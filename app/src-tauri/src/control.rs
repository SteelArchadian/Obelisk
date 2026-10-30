//! Local control API for the user's Minecraft chats (approved by the user, 2026-10-21).
//!
//! Those chats run inside the Claude app's package, where AppData is redirected and anything they start loads
//! slowly. This Obelisk, started by the user from the Start menu, does a short fixed list of jobs for them instead,
//! at full speed with the real files. The limits:
//! - it listens on 127.0.0.1 only and every request needs the token in `%USERPROFILE%\.obelisk\control.json`;
//! - a fixed list of commands, no arbitrary programs or free-form paths;
//! - games, mod changes and copies only touch test copies directly under the test bench folder; the play instance
//!   is refused (it can only be read: logs, diagnosis, and as the source of a new copy);
//! - every launch takes the bench lock and releases it when the game exits;
//! - every request is written to `<bench>\obelisk\audit.log`.
//! The test bench is this PC's own setup, described in `%USERPROFILE%\.obelisk\bench.json` (see `TestBench`).
//! Without that file only `ping`, `help` and the app bridge (`app-commands`, `app`) answer.
//! Assistants reach it through `obelisk.exe --mcp` (mcp.rs), set up from Settings → Plugins → AI assistant.

use super::NoWindow;
use obelisk_core as prism;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// This PC's test bench, from `%USERPROFILE%\.obelisk\bench.json`.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct TestBench {
    /// Folder whose direct subfolders are the test copies (plus the tool folders in NOT_COPIES).
    root: PathBuf,
    /// The Minecraft runtime (versions, libraries, assets, Java) test copies launch from.
    install: String,
    /// The test harness jar (no mouse grab, camera aim, fast quit) added to every launched copy.
    harness: String,
    /// Folder holding benchlock.py and gate.py.
    scripts: String,
    /// The Obelisk profile that counts as the play instance: readable, never launched.
    play: String,
    /// More game folders that are never launched or changed (e.g. an older play instance).
    guarded: Vec<PathBuf>,
}

fn tb() -> &'static TestBench {
    static TB: std::sync::OnceLock<TestBench> = std::sync::OnceLock::new();
    TB.get_or_init(|| std::fs::read(token_dir().join("bench.json")).ok()
        .and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default())
}

fn tb_ready() -> Result<(), String> {
    if tb().root.as_os_str().is_empty() || !tb().root.is_dir() {
        return Err(r"this PC has no test bench set up (%USERPROFILE%\.obelisk\bench.json; see the README)".into());
    }
    Ok(())
}

// For releases.rs.
pub(crate) fn bench_root() -> PathBuf {
    tb().root.clone()
}
pub(crate) fn bench_ready() -> bool {
    tb_ready().is_ok()
}
pub(crate) fn gate_script() -> String {
    format!("{}/gate.py", tb().scripts)
}
pub(crate) fn bench_jar(p: &str) -> Result<PathBuf, String> {
    mcw_jar(p)
}
pub(crate) fn game_running(dir: &Path) -> bool {
    game_running_in(dir)
}
pub(crate) fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    need(args, key)
}
pub(crate) fn opt<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    s(args, key)
}

/// The play instance's game folder (none when the bench names no play profile).
fn play_dir() -> PathBuf {
    if tb().play.is_empty() {
        return PathBuf::new();
    }
    prism::instances_dir(&prism::data_dir()).join(&tb().play).join("minecraft")
}
const DEFAULT_VERSION: &str = "neoforge-21.1.252";
/// Folders under the bench root that hold tools, not game copies.
const NOT_COPIES: &[&str] = &["scripts", "backups", "gate-template", "obelisk", "tools", "status", "maps", "mcsrc"];
/// Skipped when copying a pack (same list as gate.py).
const COPY_SKIP_DIRS: &[&str] = &["saves", "logs", "crash-reports", "screenshots", "backups", "lightning", "jfr", "debug",
    "gate-logs", "mod-backups", "voicechat_recordings", "downloads", ".stfolder", "obelisk-runs"];

#[derive(Clone, serde::Serialize)]
struct Job {
    id: u32,
    kind: String,
    copy: String,
    owner: String,
    /// queued (waiting for the bench lock), running, done, failed, killed
    state: String,
    pid: Option<u32>,
    started: u64,
    ended: Option<u64>,
    detail: String,
    /// Where the job's output or game log ends up.
    output: String,
    #[serde(skip)]
    helper_pid: Option<u32>,
    /// The game's java.exe, for jcmd next to it.
    #[serde(skip)]
    java: Option<String>,
    /// Launches: stages so far ({event, t: seconds since Java started, src}), see devrun.rs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    events: Vec<Value>,
    /// Launches: the newest once-a-second sample (CPU, memory, GPU, heap); all of them are in the samples file.
    #[serde(skip_serializing_if = "Value::is_null")]
    now: Value,
    /// Launches: what each "at" hook did ({event, after, do, t, result or error}).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    hooks: Vec<Value>,
}

#[derive(Default)]
struct Jobs {
    next: u32,
    list: HashMap<u32, Job>,
}

type Shared = Arc<(Mutex<Jobs>, Condvar)>;

fn now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn update(jobs: &Shared, id: u32, f: impl FnOnce(&mut Job)) {
    let (m, cv) = &**jobs;
    if let Some(j) = m.lock().unwrap().list.get_mut(&id) {
        f(j);
    }
    cv.notify_all();
}

// ---------------------------------------------------------------- start-up

fn random_hex(n: usize) -> String {
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(alg: isize, buf: *mut u8, len: u32, flags: u32) -> i32;
    }
    let mut buf = vec![0u8; n];
    // BCRYPT_USE_SYSTEM_PREFERRED_RNG
    let ok = unsafe { BCryptGenRandom(0, buf.as_mut_ptr(), n as u32, 2) } == 0;
    assert!(ok, "no system random source");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn token_dir() -> PathBuf {
    PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| r"C:\Users\Default".into())).join(".obelisk")
}

/// Start the server (once, from setup). Does nothing when Obelisk itself runs inside another app's package:
/// then it would have the same slow, redirected view of the files as the chats.
pub fn start(app: tauri::AppHandle) {
    if super::sandboxed() {
        return;
    }
    let Ok(listener) = TcpListener::bind("127.0.0.1:0") else { return };
    let Ok(port) = listener.local_addr().map(|a| a.port()) else { return };
    let token = random_hex(32);
    let dir = token_dir();
    let _ = std::fs::create_dir_all(&dir);
    // The folder is inside the user's profile, which only that user (and admins) can read. No ACL
    // rewriting: antivirus heuristics treat programs that strip inheritance as suspicious.
    let info = json!({ "port": port, "token": token, "pid": std::process::id(), "version": env!("CARGO_PKG_VERSION") });
    if std::fs::write(dir.join("control.json"), serde_json::to_vec_pretty(&info).unwrap()).is_err() {
        return;
    }
    if tb_ready().is_ok() {
        let _ = std::fs::create_dir_all(tb().root.join("obelisk").join("jobs"));
    }
    audit("server", &json!({ "port": port }), "started");
    let jobs: Shared = Default::default();
    let _ = ACTIVE.set(jobs.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (app, jobs, token) = (app.clone(), jobs.clone(), token.clone());
            std::thread::spawn(move || {
                let _ = handle(stream, &app, &jobs, &token);
            });
        }
    });
}

static ACTIVE: std::sync::OnceLock<Shared> = std::sync::OnceLock::new();

/// A chat's job is queued or running (restarting Obelisk now would lose it).
pub fn busy() -> bool {
    ACTIVE.get().is_some_and(|j| j.0.lock().unwrap().list.values().any(|j| j.state == "queued" || j.state == "running"))
}

/// Remove the token when Obelisk quits, so stale files don't point at a dead port.
pub fn stop() {
    let _ = std::fs::remove_file(token_dir().join("control.json"));
}

/// At most `n` characters (never splits one).
fn cut(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

fn audit(cmd: &str, args: &Value, result: &str) {
    let a = cut(&args.to_string(), 600);
    let r = cut(&result.replace('\n', " "), 300);
    let line = format!("{} {cmd} {a} -> {r}\n", stamp());
    if tb_ready().is_err() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(tb().root.join("obelisk").join("audit.log")) {
        let _ = f.write_all(line.as_bytes());
    }
}

pub(crate) fn stamp() -> String {
    // Local time via the C runtime-free route: let Windows format it.
    #[repr(C)]
    #[derive(Default)]
    struct SystemTime16 { y: u16, mo: u16, dow: u16, d: u16, h: u16, mi: u16, s: u16, ms: u16 }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocalTime(t: *mut SystemTime16);
    }
    let mut t = SystemTime16::default();
    unsafe { GetLocalTime(&mut t) };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.y, t.mo, t.d, t.h, t.mi, t.s)
}

// ---------------------------------------------------------------- HTTP

fn handle(stream: TcpStream, app: &tauri::AppHandle, jobs: &Shared, token: &str) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or("").to_string());
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    if len > 64 * 1024 {
        return reply(stream, 413, &json!({ "error": "request too large" }));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;

    // Browsers can reach 127.0.0.1 too: refuse anything that looks like a web page's request.
    let host_ok = headers.get("host").is_some_and(|h| h.starts_with("127.0.0.1:"));
    if headers.contains_key("origin") || !host_ok {
        return reply(stream, 403, &json!({ "error": "not allowed" }));
    }
    let given = headers.get("authorization").and_then(|a| a.strip_prefix("Bearer ")).unwrap_or("");
    if !same(given.as_bytes(), token.as_bytes()) {
        audit("auth", &json!({ "path": path }), "bad token");
        return reply(stream, 401, &json!({ "error": "bad token" }));
    }
    let on = super::settings_value(app).get("controlApi").and_then(|v| v.as_bool()).unwrap_or(false);
    if !on {
        return reply(stream, 403, &json!({ "error": "The user turned the control API off in Obelisk's settings." }));
    }
    let cmd = path.trim_start_matches("/v1/").trim_matches('/').to_string();
    if method != "POST" {
        return reply(stream, 405, &json!({ "error": "use POST /v1/<command>" }));
    }
    let args: Value = if body.is_empty() { json!({}) } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return reply(stream, 400, &json!({ "error": format!("bad JSON: {e}") })),
        }
    };
    let out = dispatch(&cmd, &args, jobs, app);
    let shown = if cmd == "wait" || cmd == "status" || cmd == "ping" { None } else { Some(()) };
    match &out {
        Ok(v) => {
            if shown.is_some() {
                audit(&cmd, &args, &format!("ok {}", cut(&v.to_string(), 200)));
            }
            reply(stream, 200, v)
        }
        Err(e) => {
            audit(&cmd, &args, &format!("refused: {e}"));
            reply(stream, 400, &json!({ "error": e }))
        }
    }
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn reply(mut stream: TcpStream, code: u16, v: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(v).unwrap_or_default();
    let reason = match code { 200 => "OK", 400 => "Bad Request", 401 => "Unauthorized", 403 => "Forbidden", 405 => "Method Not Allowed", _ => "Error" };
    write!(stream, "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())?;
    stream.write_all(&body)
}

// ---------------------------------------------------------------- validation

fn s<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str()).filter(|v| !v.is_empty())
}

fn need<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    s(args, key).ok_or_else(|| format!("missing \"{key}\""))
}

fn num(args: &Value, key: &str, default: u64, lo: u64, hi: u64) -> Result<u64, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_u64().filter(|n| (lo..=hi).contains(n)).ok_or_else(|| format!("\"{key}\" must be a number from {lo} to {hi}")),
    }
}

fn plain_name(n: &str, what: &str) -> Result<(), String> {
    let ok = !n.is_empty() && n.len() <= 64 && !n.starts_with('.')
        && n.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if ok { Ok(()) } else { Err(format!("{what} must be a plain folder name (letters, digits, . _ -)")) }
}

/// Free text for the bench lock and the audit log.
fn label(n: &str, what: &str) -> Result<String, String> {
    let ok = !n.is_empty() && n.len() <= 80 && n.chars().all(|c| c.is_alphanumeric() || " ._-()/:,+".contains(c));
    if ok { Ok(n.to_string()) } else { Err(format!("{what} may use letters, digits, spaces and . _ - ( ) / : , + (max 80)")) }
}

fn canon(p: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(p).ok()
}

fn is_play(p: &Path) -> bool {
    let play = play_dir();
    let mut guarded = vec![play.clone(), play.parent().unwrap_or(&play).to_path_buf()];
    guarded.extend(tb().guarded.iter().cloned());
    canon(p).is_some_and(|c| guarded.iter().any(|x| !x.as_os_str().is_empty() && canon(x).as_ref() == Some(&c)))
}

/// A test copy: a game folder directly in the test bench (not a link to somewhere else, never the play instance).
fn test_copy(name: &str) -> Result<PathBuf, String> {
    plain_name(name, "copy")?;
    if NOT_COPIES.contains(&name.to_ascii_lowercase().as_str()) {
        return Err(format!("{name} isn't a game copy"));
    }
    let dir = tb().root.as_path().join(name);
    let c = canon(&dir).ok_or_else(|| format!("no copy called {name} in the test bench"))?;
    let root = canon(tb().root.as_path()).ok_or("the test bench folder is missing")?;
    if c.parent() != Some(root.as_path()) || is_play(&c) {
        return Err(format!("{name} points outside the test bench; refused"));
    }
    if !c.join("mods").is_dir() {
        return Err(format!("{name} has no mods folder, so it isn't a game copy"));
    }
    Ok(dir)
}

/// Somewhere to read from: a test copy, or "play" for the play instance (read-only commands only).
fn readable(name: &str) -> Result<PathBuf, String> {
    if name == "play" { Ok(play_dir()) } else { test_copy(name) }
}

/// Somewhere to change mods: a test copy, or "play" (user, 2026-10-24: experimental updates are fine there
/// while backups run), after a safety backup so every change can be tracked and undone in Obelisk's Backups.
fn changeable(name: &str, what: &str) -> Result<PathBuf, String> {
    if name != "play" {
        return test_copy(name);
    }
    let dir = play_dir();
    prism::history::create(&dir, "safety", &format!("Before {what} (chat via Obelisk MCP)"), true)?;
    Ok(dir)
}

/// After a change to the play instance: back up the new state, so the changelog says which chat change it was.
fn changed(dir: &Path, what: &str) {
    if *dir == play_dir() {
        let _ = prism::history::create(dir, "safety", &format!("{what} (chat via Obelisk MCP)"), true);
    }
}

/// A file the user's chats built: must be a .jar in the test bench.
fn mcw_jar(p: &str) -> Result<PathBuf, String> {
    let c = canon(Path::new(p)).ok_or_else(|| format!("{p} doesn't exist"))?;
    let root = canon(tb().root.as_path()).ok_or("the test bench folder is missing")?;
    let is_jar = c.extension().is_some_and(|e| e.eq_ignore_ascii_case("jar"));
    if !c.starts_with(&root) || !is_jar || !c.is_file() {
        return Err("the jar must be a .jar file in the test bench".into());
    }
    Ok(c)
}

fn jvm_props(args: &Value) -> Result<Vec<String>, String> {
    let Some(list) = args.get("jvm") else { return Ok(vec![]) };
    let list = list.as_array().ok_or("\"jvm\" must be a list of -Dmod.key=value strings")?;
    let mut out = vec![];
    for v in list {
        let a = v.as_str().ok_or("\"jvm\" entries must be strings")?;
        let Some((key, value)) = a.strip_prefix("-D").and_then(|r| r.split_once('=')) else {
            return Err(format!("{a}: only -Dmod.key=value system properties are allowed"));
        };
        let first = key.split('.').next().unwrap_or("").to_ascii_lowercase();
        let blocked = ["java", "javax", "jdk", "sun", "com", "org", "jna", "jnr", "log4j", "file", "user", "os", "line", "path", "fml", "neoforge", "forge", "mixin"];
        let key_ok = key.contains('.') && key.len() <= 80 && key.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
        let value_ok = value.len() <= 200 && !value.chars().any(|c| c.is_control() || c == '"');
        if blocked.contains(&first.as_str()) || !key_ok || !value_ok {
            return Err(format!("{a}: not an allowed property (mod settings like -Doptimal.x, -Dlightning.capture, -Dpackbench.scene)"));
        }
        out.push(a.to_string());
    }
    Ok(out)
}

fn game_running_in(dir: &Path) -> bool {
    let want = canon(dir).map(|c| c.to_string_lossy().trim_start_matches(r"\\?\").replace('\\', "/").to_ascii_lowercase());
    let Some(want) = want else { return false };
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::All, true, sysinfo::ProcessRefreshKind::new().with_cmd(sysinfo::UpdateKind::Always));
    sys.processes().values().any(|p| {
        p.name().to_string_lossy().to_ascii_lowercase().starts_with("java")
            && p.cmd().iter().any(|a| a.to_string_lossy().replace('\\', "/").to_ascii_lowercase().trim_end_matches('/') == want)
    })
}

// ---------------------------------------------------------------- commands

fn dispatch(cmd: &str, args: &Value, jobs: &Shared, app: &tauri::AppHandle) -> Result<Value, String> {
    if !matches!(cmd, "ping" | "help" | "app-commands" | "app") {
        tb_ready()?;
    }
    match cmd {
        "ping" => Ok(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") })),
        "help" => Ok(json!({ "commands": ["ping", "copies", "launch", "gate", "status", "wait", "kill", "mods", "mod-enable",
            "mod-disable", "mod-add", "mod-remove", "mod-install", "updates", "mod-update", "duplicates", "reports", "diagnose", "log",
            "screenshots", "lightning", "timeline", "copy-pack", "app-commands", "app", "release-submit", "release-status", "release-promote",
            "release-rollback", "release-optout", "errors"],
            "note": "Every launch waits for the shared bench lock; use wait instead of polling status." })),
        "copies" => copies(),
        // Everything the app itself can do, run live through its window (bridge.rs).
        "app-commands" => Ok(super::bridge::commands()),
        "app" => super::bridge::call(app, args),
        "launch" => launch(args, jobs),
        "gate" => gate(args, jobs),
        "errors" => Ok(super::errlog::recent(num(args, "limit", 30, 1, 200)? as usize, args.get("since").and_then(|v| v.as_u64()).unwrap_or(0))),
        "release-submit" | "release-status" | "release-promote" | "release-rollback" | "release-optout" => super::releases::command(cmd, args),
        "status" => {
            let (m, _) = &**jobs;
            let l = m.lock().unwrap();
            let mut v: Vec<_> = l.list.values().cloned().collect();
            v.sort_by_key(|j| std::cmp::Reverse(j.id));
            if let Some(id) = args.get("job").and_then(|j| j.as_u64()) {
                v.retain(|j| j.id as u64 == id);
            }
            Ok(json!({ "jobs": v }))
        }
        "wait" => wait(args, jobs),
        "kill" => kill(args, jobs),
        "heap" => heap(args, jobs),
        // What the running Obelisk sees (a chat's own view of AppData is a redirected copy).
        "data-dir-set" => {
            let dir = need(args, "dir")?.to_string();
            let app = app.clone();
            // Reply first; the move closes and restarts Obelisk.
            std::thread::spawn(move || { std::thread::sleep(Duration::from_secs(1)); let _ = super::datadir::data_dir_set(app, dir); });
            Ok(json!({ "restarting": true }))
        }
        "data-dir" => Ok(json!({ "root": super::datadir::root(), "default": super::datadir::default_root(),
            "pointer": std::fs::read_to_string(super::datadir::default_root().join("data-location.txt")).ok(),
            "log": std::fs::read_to_string(super::datadir::default_root().join("data-location.log")).ok() })),
        "mods" => {
            let dir = readable(need(args, "copy")?)?;
            let items = prism::list_content(&dir, "mods");
            let rows: Vec<_> = items.iter().map(|i| json!({ "file": i.file_name, "enabled": i.enabled, "name": i.name, "version": i.version })).collect();
            Ok(json!({ "mods": rows, "duplicates": prism::dupes::find(&dir) }))
        }
        "duplicates" => Ok(json!({ "duplicates": prism::dupes::find(&readable(need(args, "copy")?)?),
            "note": "Two enabled jars with the same mod id (often one from Modrinth, one from CurseForge) stop the game starting; disable or remove one." })),
        "updates" => {
            let dir = readable(need(args, "copy")?)?;
            let (loader, mc) = pack_of(args)?;
            let mr = prism::modrinth::Client::new()?;
            let infos = prism::updates::check(&dir, &super::datadir::root().join("hashes"), &mr, super::cf_client().as_ref(), &loader, &mc)?;
            let rows: Vec<_> = infos.iter().filter(|i| i.update.is_some()).map(|i| json!({ "file": i.file_name, "kind": i.kind,
                "provider": i.provider, "project": i.project_id, "update": i.update })).collect();
            let unknown: Vec<_> = infos.iter().filter(|i| i.provider.is_empty()).map(|i| &i.file_name).collect();
            Ok(json!({ "loader": loader, "minecraft": mc, "updates": rows, "not_on_either_site": unknown }))
        }
        "mod-update" => {
            let file = need(args, "file")?;
            let dir = changeable(need(args, "copy")?, &format!("mod-update {file}"))?;
            if game_running_in(&dir) {
                return Err("a game is running in that copy; stop it first".into());
            }
            let (loader, mc) = pack_of(args)?;
            let mr = prism::modrinth::Client::new()?;
            let cf = super::cf_client();
            let infos = prism::updates::check(&dir, &super::datadir::root().join("hashes"), &mr, cf.as_ref(), &loader, &mc)?;
            let info = infos.iter().find(|i| i.file_name == file).ok_or_else(|| format!("{file} isn't in that copy"))?;
            let up = info.update.as_ref().ok_or_else(|| format!("{file} is up to date (or not on Modrinth/CurseForge)"))?;
            if up.blocked {
                return Err(format!("CurseForge only allows {} to be downloaded on its website; ask the user", up.file_name));
            }
            if info.provider == "modrinth" {
                let project_type = match info.kind.as_str() { "resourcepacks" => "resourcepack", "shaderpacks" => "shader", _ => "mod" };
                let req = prism::modrinth::InstallRequest { project_id: info.project_id.clone(), project_type: project_type.into(),
                    version_id: Some(up.id.clone()), loader_override: None, force: false };
                let existing = vec![prism::modrinth::Installed { project_id: info.project_id.clone(), version_id: String::new(), kind: info.kind.clone(), file_name: file.into() }];
                mr.install(&dir, &req, &loader, &mc, &existing)?;
            } else {
                let cf = cf.ok_or("no CurseForge API key in Obelisk's Settings")?;
                let (m, f) = (info.project_id.parse().map_err(|_| "bad project id")?, up.id.parse().map_err(|_| "bad file id")?);
                if cf.replace(&dir, &info.kind, file, m, f)?.is_some() {
                    return Err("CurseForge only allows this file on its website; ask the user".into());
                }
            }
            changed(&dir, &format!("mod-update {file} -> {}", up.file_name));
            Ok(json!({ "updated": file, "to": up.file_name, "duplicates": prism::dupes::find(&dir) }))
        }
        "mod-install" => {
            // "AANobbMI" / "sodium" (Modrinth id or slug) or "cf:238222" (CurseForge project id).
            let project = need(args, "project")?.to_string();
            let dir = changeable(need(args, "copy")?, &format!("mod-install {project}"))?;
            if game_running_in(&dir) {
                return Err("a game is running in that copy; stop it first".into());
            }
            let (loader, mc) = pack_of(args)?;
            let version = s(args, "version").map(str::to_string);
            let mr = prism::modrinth::Client::new()?;
            let cache = super::datadir::root().join("hashes");
            let names = if let Some(id) = project.strip_prefix("cf:") {
                let cf = super::cf_client().ok_or("no CurseForge API key in Obelisk's Settings")?;
                let id: u64 = id.parse().map_err(|_| "cf: needs a number")?;
                let file_id = version.map(|v| v.parse::<u64>().map_err(|_| "version must be a CurseForge file id")).transpose()?;
                let known = prism::updates::check(&dir, &cache, &mr, Some(&cf), &loader, &mc)?
                    .into_iter().filter(|i| i.provider == "curseforge").filter_map(|i| i.project_id.parse().ok()).collect();
                match cf.install(&dir, id, file_id, &loader, &mc, &known)? {
                    prism::curseforge::InstallOutcome::Done { names } => names,
                    prism::curseforge::InstallOutcome::Blocked { name, page, .. } =>
                        return Err(format!("CurseForge only allows {name} to be downloaded on its website ({page}); ask the user")),
                }
            } else {
                let existing = mr.installed(&dir, &cache)?;
                let req = prism::modrinth::InstallRequest { project_id: project.clone(), project_type: "mod".into(), version_id: version, loader_override: None, force: false };
                mr.install(&dir, &req, &loader, &mc, &existing)?
            };
            changed(&dir, &format!("mod-install {project}"));
            Ok(json!({ "installed": names, "duplicates": prism::dupes::find(&dir) }))
        }
        "mod-enable" | "mod-disable" => {
            let file = need(args, "file")?;
            let dir = changeable(need(args, "copy")?, &format!("{cmd} {file}"))?;
            if game_running_in(&dir) {
                return Err("a game is running in that copy; stop it first".into());
            }
            if cmd == "mod-enable" {
                let ids = prism::dupes::own_ids(&dir.join("mods").join(file));
                if let Some((id, other)) = prism::dupes::clash(&prism::dupes::index(&dir.join("mods")), &ids, file) {
                    return Err(format!("{other} already provides {id}; enabling {file} too would stop the game starting"));
                }
            }
            let f = prism::set_enabled(&dir, "mods", file, cmd == "mod-enable")?;
            changed(&dir, &format!("{cmd} {file}"));
            Ok(json!({ "file": f }))
        }
        "mod-add" => {
            let jar = mcw_jar(need(args, "jar")?)?;
            let name = jar.file_name().unwrap().to_owned();
            let dir = changeable(need(args, "copy")?, &format!("mod-add {}", name.to_string_lossy()))?;
            if game_running_in(&dir) {
                return Err("a game is running in that copy; stop it first".into());
            }
            let ids = prism::dupes::own_ids(&jar);
            if let Some((id, other)) = prism::dupes::clash(&prism::dupes::index(&dir.join("mods")), &ids, &name.to_string_lossy()) {
                return Err(format!("{other} already provides {id}; remove or disable it first (two copies stop the game starting)"));
            }
            std::fs::copy(&jar, dir.join("mods").join(&name)).map_err(|e| e.to_string())?;
            changed(&dir, &format!("mod-add {}", name.to_string_lossy()));
            Ok(json!({ "file": name.to_string_lossy() }))
        }
        "mod-remove" => {
            let file = need(args, "file")?;
            if file.contains(['/', '\\']) || file.starts_with('.') || !(file.ends_with(".jar") || file.ends_with(".jar.disabled")) {
                return Err("\"file\" must be a jar file name from the mods list".into());
            }
            let dir = changeable(need(args, "copy")?, &format!("mod-remove {file}"))?;
            if game_running_in(&dir) {
                return Err("a game is running in that copy; stop it first".into());
            }
            let src = dir.join("mods").join(file);
            if !src.is_file() {
                return Err(format!("{file} isn't in mods"));
            }
            // Never deleted: moved aside so it can be put back.
            let aside = dir.join("mods-removed-obelisk");
            std::fs::create_dir_all(&aside).map_err(|e| e.to_string())?;
            std::fs::rename(&src, aside.join(file)).map_err(|e| e.to_string())?;
            changed(&dir, &format!("mod-remove {file}"));
            Ok(json!({ "moved_to": aside.join(file) }))
        }
        "reports" => {
            let dir = readable(need(args, "copy")?)?;
            Ok(serde_json::to_value(prism::crash::list(&dir)).unwrap_or_default())
        }
        "diagnose" => {
            let dir = readable(need(args, "copy")?)?;
            let file = s(args, "file").unwrap_or("logs/latest.log");
            let mut d = prism::crash::diagnose(&dir, file)?;
            if let Some((code, lines)) = failed_run(&dir) {
                // The game died before it could log why; the JVM's own output says.
                let hit = lines.iter().position(|l| l.contains("Exception") || l.contains("Error:") || l.contains("Error ")).unwrap_or(0);
                let line = lines.get(hit).cloned().unwrap_or_default();
                d.headline = format!("The game exited with code {code}: {}", explain_java_error(&line).unwrap_or_else(|| line.clone()));
                d.error = line;
                d.excerpt = lines.into_iter().skip(hit).take(40).collect();
            }
            d.excerpt.truncate(40);
            Ok(serde_json::to_value(d).unwrap_or_default())
        }
        "log" => log(args),
        "screenshots" => {
            let dir = readable(need(args, "copy")?)?;
            let n = num(args, "count", 5, 1, 50)? as usize;
            let mut shots = prism::list_screenshots(&dir);
            shots.truncate(n);
            Ok(json!({ "screenshots": shots }))
        }
        "lightning" => {
            let dir = readable(need(args, "copy")?)?;
            let n = num(args, "count", 3, 1, 20)? as usize;
            // Summaries as the Diagnosis tab shows them (FPS, slow frames, memory, GC), plus the raw files for detail.
            Ok(json!({ "runs": prism::lightning::runs(&dir, n), "results": newest_named(&dir.join("lightning"), "results.json", n) }))
        }
        "timeline" => {
            let dir = readable(need(args, "copy")?)?;
            let n = num(args, "count", 5, 1, 50)? as usize;
            Ok(json!({ "sessions": prism::timeline::sessions(&dir, n).iter().map(|s| json!({
                "session": s, "phases": prism::timeline::phases(s) })).collect::<Vec<_>>() }))
        }
        "copy-pack" => copy_pack(args, jobs),
        _ => Err(format!("unknown command \"{cmd}\"; try help")),
    }
}

/// Loader and Minecraft version for installs and update checks: given, or the play instance's
/// (every test copy is a copy of Foundation unless said otherwise).
fn pack_of(args: &Value) -> Result<(String, String), String> {
    let play = prism::list_instances(&prism::data_dir()).into_iter().find(|i| i.id == "Foundation");
    let loader = s(args, "loader").map(str::to_string).or_else(|| play.as_ref().map(|p| p.loader.clone()));
    let mc = s(args, "minecraft").map(str::to_string).or_else(|| play.as_ref().map(|p| p.minecraft.clone()));
    Ok((loader.ok_or("pass loader (e.g. neoforge)")?, mc.ok_or("pass minecraft (e.g. 1.21.1)")?))
}

fn copies() -> Result<Value, String> {
    let mut out = vec![];
    for e in std::fs::read_dir(&tb().root).map_err(|e| e.to_string())?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if test_copy(&name).is_ok() && e.path().join("config").is_dir() {
            out.push(json!({ "copy": name, "running": game_running_in(&e.path()) }));
        }
    }
    Ok(json!({ "copies": out, "play": "readable as \"play\" (logs, diagnose, mods list, copy-pack source); never launched or changed" }))
}

fn newest_named(dir: &Path, file: &str, n: usize) -> Vec<PathBuf> {
    let mut found = vec![];
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() && depth < 3 {
                stack.push((p, depth + 1));
            } else if e.file_name() == file {
                let t = e.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                found.push((t, p));
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().take(n).map(|(_, p)| p).collect()
}

fn log(args: &Value) -> Result<Value, String> {
    let dir = readable(need(args, "copy")?)?;
    let file = s(args, "file").unwrap_or("logs/latest.log");
    let text = if file.starts_with("logs/") && file != "logs/latest.log" {
        prism::read_log_tail(&dir, file.trim_start_matches("logs/"), 8 * 1024 * 1024)?
    } else {
        prism::crash::read(&dir, file)?
    };
    let grep = s(args, "grep").map(|g| g.to_lowercase());
    let tail = num(args, "tail", 200, 1, 2000)? as usize;
    let lines: Vec<&str> = text.lines().filter(|l| grep.as_ref().is_none_or(|g| l.to_lowercase().contains(g))).collect();
    let total = lines.len();
    let shown: Vec<&str> = lines[total.saturating_sub(tail)..].to_vec();
    Ok(json!({ "file": file, "matched": total, "lines": shown }))
}

fn wait(args: &Value, jobs: &Shared) -> Result<Value, String> {
    let id = args.get("job").and_then(|j| j.as_u64()).ok_or("missing \"job\"")? as u32;
    let secs = num(args, "timeout", 600, 1, 3600)?;
    let (m, cv) = &**jobs;
    let deadline = Instant::now() + Duration::from_secs(secs);
    let until = s(args, "until");
    if let Some(u) = until.filter(|u| !super::devrun::STAGES.contains(u) && !u.starts_with("pressure")) {
        return Err(format!("until: {u} isn't a stage; use one of {}", super::devrun::STAGES.join(", ")));
    }
    let mut l = m.lock().unwrap();
    loop {
        let j = l.list.get(&id).ok_or("no such job")?;
        if !matches!(j.state.as_str(), "queued" | "running") || until.is_some_and(|u| j.events.iter().any(|e| e["event"] == u)) {
            return Ok(json!(j));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(json!(j));
        }
        l = cv.wait_timeout(l, left).unwrap().0;
    }
}

fn kill(args: &Value, jobs: &Shared) -> Result<Value, String> {
    let id = args.get("job").and_then(|j| j.as_u64()).ok_or("missing \"job\"")? as u32;
    let (m, _) = &**jobs;
    let j = m.lock().unwrap().list.get(&id).cloned().ok_or("no such job")?;
    let mut killed = vec![];
    for pid in [j.pid, j.helper_pid].into_iter().flatten() {
        if super::kill_tree(pid) {
            killed.push(pid);
        }
    }
    update(jobs, id, |j| j.detail = KILLED.into());
    Ok(json!({ "job": id, "killed": killed }))
}

const KILLED: &str = "kill requested";

/// A running game's heap through jcmd: "histogram" (live objects per class: count and shallow bytes; forces a
/// full GC, takes a few seconds, fine to repeat over time) or "dump" (an .hprof of live objects for MAT or
/// VisualVM; pauses the game for the whole write and is about as big as the used heap). Files go to <copy>/jfr/.
fn heap(args: &Value, jobs: &Shared) -> Result<Value, String> {
    let id = args.get("job").and_then(|j| j.as_u64()).ok_or("missing \"job\"")? as u32;
    let kind = args.get("kind").and_then(|k| k.as_str()).unwrap_or("histogram");
    let j = jobs.0.lock().unwrap().list.get(&id).cloned().ok_or("no such job")?;
    let (Some(pid), Some(java), "running") = (j.pid, j.java.as_ref(), j.state.as_str()) else {
        return Err("that job has no running game".into());
    };
    snapshot(&super::devrun::jcmd_for(java)?, pid, &readable(&j.copy)?.join("jfr"), kind)
}

fn snapshot(jcmd: &Path, pid: u32, out: &Path, kind: &str) -> Result<Value, String> {
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let run = |a: &[&str]| -> Result<String, String> {
        let o = Command::new(jcmd).arg(pid.to_string()).args(a).creation_flags_no_window().output().map_err(|e| format!("jcmd: {e}"))?;
        let text = String::from_utf8_lossy(&o.stdout).into_owned();
        if o.status.success() { Ok(text) } else { Err(format!("jcmd failed: {}", text.lines().last().unwrap_or(""))) }
    };
    match kind {
        "histogram" => {
            let text = run(&["GC.class_histogram"])?;
            let file = out.join(format!("histo-{}.txt", now()));
            std::fs::write(&file, &text).map_err(|e| e.to_string())?;
            let top: Vec<&str> = text.lines().take(43).collect();
            Ok(json!({ "file": file, "top": top.join("\n"), "total": text.lines().last().unwrap_or("") }))
        }
        "dump" => {
            let file = out.join(format!("heap-{}.hprof", now()));
            let t0 = Instant::now();
            run(&["GC.heap_dump", &file.to_string_lossy()])?;
            let mb = std::fs::metadata(&file).map(|m| m.len() / 1_048_576).unwrap_or(0);
            Ok(json!({ "file": file, "mb": mb, "seconds": t0.elapsed().as_secs() }))
        }
        "threads" => {
            // Three thread dumps 1.5 s apart: a thread whose top frames never move is where the game hangs.
            let mut dumps = vec![];
            for i in 0..3 {
                if i > 0 { std::thread::sleep(Duration::from_millis(1500)); }
                dumps.push(run(&["Thread.print"])?);
            }
            let file = out.join(format!("threads-{}.txt", now()));
            std::fs::write(&file, dumps.join("\n\n==== next sample ====\n\n")).map_err(|e| e.to_string())?;
            let parsed: Vec<Vec<JThread>> = dumps.iter().map(|d| parse_threads(d)).collect();
            Ok(json!({ "file": file, "threads": summarize_threads(&parsed) }))
        }
        _ => Err("kind must be histogram, dump or threads".into()),
    }
}

/// One thread from a jcmd Thread.print.
struct JThread {
    name: String,
    state: String,
    frames: Vec<String>,
}

fn parse_threads(text: &str) -> Vec<JThread> {
    let mut out: Vec<JThread> = vec![];
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('"') {
            let name = rest.split('"').next().unwrap_or("").to_string();
            out.push(JThread { name, state: String::new(), frames: vec![] });
        } else if let Some(t) = out.last_mut() {
            let l = line.trim();
            if let Some(st) = l.strip_prefix("java.lang.Thread.State: ") {
                t.state = st.split_whitespace().next().unwrap_or("").to_string();
            } else if let Some(f) = l.strip_prefix("at ") {
                t.frames.push(f.split('(').next().unwrap_or(f).to_string());
            }
        }
    }
    out
}

/// The threads that matter for a hang (render, server, main, loading workers that are busy), with
/// their top frames, whether they stayed on the same frames in every sample, and which mods' code
/// they are in.
fn summarize_threads(samples: &[Vec<JThread>]) -> Vec<Value> {
    let Some(last) = samples.last() else { return vec![] };
    let key = |n: &str| ["Render thread", "Server thread", "main"].contains(&n);
    let busy_worker = |t: &JThread| t.state == "RUNNABLE" && (t.name.contains("Worker") || t.name.contains("modloading") || t.name.contains("Bootstrap"))
        && t.frames.first().is_some_and(|f| !f.starts_with("java.") && !f.starts_with("jdk.") && !f.starts_with("sun."));
    let top = |t: &JThread| t.frames.iter().take(6).cloned().collect::<Vec<_>>();
    last.iter().filter(|t| key(&t.name) || busy_worker(t)).take(10).map(|t| {
        let same = samples.iter().all(|s| s.iter().any(|o| o.name == t.name && top(o) == top(t))) && !t.frames.is_empty();
        let mut code: Vec<String> = vec![];
        for f in &t.frames {
            let parts: Vec<&str> = f.split('.').collect();
            if parts.len() < 3 || ["java", "jdk", "sun", "com.sun"].contains(&parts[0]) { continue }
            let root = parts[..2.min(parts.len() - 1)].join(".");
            if !code.contains(&root) { code.push(root) }
        }
        json!({ "thread": t.name, "state": t.state, "stuck_same_place": same,
            "top": t.frames.iter().take(14).collect::<Vec<_>>(), "packages": code.into_iter().take(12).collect::<Vec<_>>() })
    }).collect()
}

/// A launch's "at" hook: when `event` has happened and `after` more seconds have passed, do `action`.
struct Hook {
    event: String,
    after: f64,
    action: String,
    fired: bool,
}

const HOOK_ACTIONS: &[&str] = &["histogram", "dump", "threads", "kill"];

fn plan_hooks(args: &Value) -> Result<Vec<Hook>, String> {
    let Some(list) = args.get("at") else { return Ok(vec![]) };
    // A client with an old tool list sends it as JSON text.
    let parsed = list.as_str().and_then(|t| serde_json::from_str::<Value>(t).ok());
    let list = parsed.as_ref().unwrap_or(list).as_array().ok_or("\"at\" must be a list")?;
    if list.len() > 20 { return Err("at most 20 \"at\" hooks".into()); }
    list.iter().map(|h| {
        let event = h.get("event").and_then(Value::as_str).ok_or("each \"at\" needs an event")?;
        if !super::devrun::STAGES.contains(&event) && !event.starts_with("pressure") {
            return Err(format!("at: {event} isn't a stage; use one of {}", super::devrun::STAGES.join(", ")));
        }
        let action = h.get("do").and_then(Value::as_str).ok_or("each \"at\" needs a \"do\"")?;
        if !HOOK_ACTIONS.contains(&action) {
            return Err(format!("at: do must be one of {}", HOOK_ACTIONS.join(", ")));
        }
        let after = h.get("after").and_then(Value::as_f64).unwrap_or(0.0);
        if !(0.0..=3600.0).contains(&after) { return Err("at: after is 0-3600 seconds".into()); }
        Ok(Hook { event: event.into(), after, action: action.into(), fired: false })
    }).collect()
}

/// Final state, unless someone killed the job.
fn finish(jobs: &Shared, id: u32, state: &str, detail: String) {
    update(jobs, id, |j| {
        let killed = j.detail == KILLED;
        j.state = if killed { "killed".into() } else { state.into() };
        j.detail = if killed { format!("killed on request ({detail})") } else { detail };
        j.ended = Some(now());
    });
}

fn new_job(jobs: &Shared, kind: &str, copy: &str, owner: &str, output: String) -> u32 {
    let (m, cv) = &**jobs;
    let mut l = m.lock().unwrap();
    l.next += 1;
    let id = l.next;
    l.list.insert(id, Job { id, kind: kind.into(), copy: copy.into(), owner: owner.into(), state: "queued".into(), pid: None,
        started: now(), ended: None, detail: String::new(), output, helper_pid: None, java: None, events: vec![], now: Value::Null, hooks: vec![] });
    cv.notify_all();
    id
}

pub(crate) fn python() -> Command {
    let mut c = Command::new("python");
    c.arg("-I").creation_flags_no_window();
    c
}

fn bench(args: &[&str]) -> Command {
    let mut c = python();
    c.arg(format!("{}/benchlock.py", tb().scripts)).args(args);
    c
}

// ---------------------------------------------------------------- launch

/// Test rules for every test bench copy (user, 2026-10-24): the game keeps running when the user's mouse or keys go
/// elsewhere, and Absolute AFK Cinema can't take over the camera after 15 s idle. Only ever called on test copies.
fn test_rules(dir: &Path) {
    let options = dir.join("options.txt");
    if !options.exists() {
        let _ = std::fs::write(&options, ""); // a copy without one would stop at the first-start accessibility screen
    }
    set_option(&options, "pauseOnLostFocus", "false");
    set_option(&options, "onboardAccessibility", "false");
    // Windowed: a borderless or fullscreen game copied from the play instance covers the desktop and
    // glitches Wallpaper Engine while it's parked off-screen.
    let _ = prism::window::set(dir, prism::window::Mode::Windowed);
    afk_cinema_off(dir);
    essential_no_update(dir);
}

/// Essential's loader asks in a window of its own whether to update Essential ("with-prompt"), and a quiet bench
/// launch waits on that window forever. Copies never update it: autoUpdate=false in its own config file
/// (essential/essential-loader.properties), every other line kept.
fn essential_no_update(dir: &Path) {
    let installed = dir.join("essential").is_dir() || std::fs::read_dir(dir.join("mods")).into_iter().flatten().flatten()
        .any(|e| e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("essential"));
    if !installed { return }
    let file = dir.join("essential").join("essential-loader.properties");
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().filter(|l| l.split('=').next().map(str::trim) != Some("autoUpdate")).map(String::from).collect();
    lines.push("autoUpdate=false".into());
    let _ = std::fs::create_dir_all(dir.join("essential"));
    let _ = std::fs::write(&file, lines.join("
") + "
");
}

/// Absolute AFK Cinema's master switch is `general.enabled` in a commented JSON file, so it's switched as text
/// (comments and every other setting kept). With the mod installed but no config yet, a file holding just the
/// switch is written first; the mod fills in the rest of its defaults around it.
fn afk_cinema_off(dir: &Path) {
    let afk = dir.join("config").join("absoluteafkcinema-client.json");
    match std::fs::read_to_string(&afk) {
        Ok(text) => {
            if let Some(off) = afk_switched_off(&text) {
                let _ = std::fs::write(&afk, off);
            }
        }
        Err(_) => {
            let installed = std::fs::read_dir(dir.join("mods")).into_iter().flatten().flatten()
                .any(|e| e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("absoluteafkcinema"));
            if installed {
                let _ = std::fs::create_dir_all(dir.join("config"));
                let _ = std::fs::write(&afk, "{
  \"general\": {
    \"enabled\": false
  }
}
");
            }
        }
    }
}

/// The config text with `general.enabled` set to false, or None when it already is (or isn't there).
fn afk_switched_off(text: &str) -> Option<String> {
    let general = text.find("\"general\"")?;
    let key = general + text[general..].find("\"enabled\"")?;
    let colon = key + text[key..].find(':')?;
    let rest = &text[colon + 1..];
    let start = colon + 1 + (rest.len() - rest.trim_start().len());
    if !text[start..].starts_with("true") {
        return None;
    }
    Some(format!("{}false{}", &text[..start], &text[start + 4..]))
}

fn set_option(path: &Path, key: &str, value: &str) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let mut found = false;
    let mut out: Vec<String> = text.lines().map(|l| {
        if l.starts_with(&format!("{key}:")) { found = true; format!("{key}:{value}") } else { l.to_string() }
    }).collect();
    if !found {
        out.push(format!("{key}:{value}"));
    }
    let _ = std::fs::write(path, out.join("\n") + "\n");
}

fn rules_ok(rules: Option<&Value>) -> bool {
    let Some(rules) = rules.and_then(|r| r.as_array()) else { return true };
    let mut allow = false;
    for r in rules {
        let os_ok = r.get("os").and_then(|o| o.get("name")).and_then(|n| n.as_str()).is_none_or(|n| n == "windows");
        let feature = r.get("features").is_some();
        if os_ok && !feature {
            allow = r.get("action").and_then(|a| a.as_str()) == Some("allow");
        }
    }
    allow
}

fn lib_paths(libs: &Value, lib: &str) -> Vec<String> {
    let mut out = vec![];
    for l in libs.as_array().into_iter().flatten() {
        if !rules_ok(l.get("rules")) {
            continue;
        }
        if let Some(p) = l.pointer("/downloads/artifact/path").and_then(|p| p.as_str()) {
            out.push(format!("{lib}/{p}"));
        } else if let (Some(name), None) = (l.get("name").and_then(|n| n.as_str()), l.get("downloads")) {
            let parts: Vec<&str> = name.split(':').collect();
            if parts.len() >= 3 {
                let (g, a, v) = (parts[0].replace('.', "/"), parts[1], parts[2]);
                out.push(format!("{lib}/{g}/{a}/{v}/{a}-{v}.jar"));
            }
        }
    }
    out
}

/// The NeoForge version a copy was made for (CurseForge's minecraftinstance.json), else the pack's.
fn loader_version(dir: &Path) -> String {
    std::fs::read(dir.join("minecraftinstance.json")).ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.pointer("/baseModLoader/name").and_then(|n| n.as_str()).map(String::from))
        .filter(|n| n.starts_with("neoforge-") && n.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c)))
        .unwrap_or_else(|| DEFAULT_VERSION.into())
}

/// The same command gate.py builds, for any copy.
fn game_command(dir: &Path, extra: &[String]) -> Result<(String, Vec<String>), String> {
    let version = loader_version(dir);
    let install = tb().install.replace(std::path::MAIN_SEPARATOR, "/");
    let lib = format!("{install}/libraries");
    let read = |p: String| -> Result<Value, String> {
        serde_json::from_slice(&std::fs::read(&p).map_err(|e| format!("{p}: {e}"))?).map_err(|e| e.to_string())
    };
    let neo = read(format!("{install}/versions/{version}/{version}.json"))?;
    let van = read(format!("{install}/versions/1.21.1/1.21.1.json"))?;
    let mut cp: Vec<String> = vec![];
    for p in lib_paths(&neo["libraries"], &lib).into_iter().chain(lib_paths(&van["libraries"], &lib)) {
        if !cp.contains(&p) && Path::new(&p).exists() {
            cp.push(p);
        }
    }
    let copy = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    override_libraries(&mut cp, &copy_source(&copy).libraries, &lib)?;
    cp.push(format!("{install}/versions/{version}/{version}.jar"));
    let game_dir = dir.to_string_lossy().replace('\\', "/");
    let subs = [("${library_directory}", lib.clone()), ("${classpath_separator}", ";".into()), ("${version_name}", version.clone()),
        ("${natives_directory}", format!("{install}/natives/{version}")), ("${launcher_name}", "obelisk".into()),
        ("${launcher_version}", "1".into()), ("${classpath}", cp.join(";"))];
    let sub = |s: &str| subs.iter().fold(s.to_string(), |acc, (k, v)| acc.replace(k, v));
    let mut a: Vec<String> = ["-Xmx6000m", "-Xms256m", "-XX:+UseZGC", "-XX:+ZGenerational", "-Dvoxy.experimentalTemporalMdi=true",
        "-Dtestharness.background=true"].iter().map(|s| s.to_string()).collect();
    a.extend(extra.iter().cloned());
    for x in van.pointer("/arguments/jvm").and_then(|v| v.as_array()).into_iter().flatten() {
        if let Some(s) = x.as_str() {
            a.push(sub(s));
        } else if rules_ok(x.get("rules")) {
            match &x["value"] {
                Value::String(s) => a.push(sub(s)),
                Value::Array(v) => a.extend(v.iter().filter_map(|s| s.as_str()).map(sub)),
                _ => {}
            }
        }
    }
    a.extend(neo.pointer("/arguments/jvm").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|s| s.as_str()).map(sub));
    a.push(neo["mainClass"].as_str().ok_or("no mainClass")?.to_string());
    let uuid = format!("{:032x}", u128::from_str_radix(&prism::sha1_hex(game_dir.as_bytes())[..32], 16).unwrap_or(1));
    a.extend(["--username", "ObeliskTest", "--version", &version, "--gameDir", &game_dir, "--assetsDir", &format!("{install}/assets"),
        "--assetIndex", van.pointer("/assetIndex/id").and_then(|v| v.as_str()).unwrap_or("17"), "--uuid", &uuid,
        "--accessToken", "0", "--clientId", "0", "--xuid", "0", "--userType", "legacy", "--versionType", "release"].map(String::from));
    a.extend(neo.pointer("/arguments/game").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|s| s.as_str()).map(String::from));
    Ok((format!("{install}/java/java-runtime-delta/bin/java.exe"), a))
}

fn launch(args: &Value, jobs: &Shared) -> Result<Value, String> {
    let name = need(args, "copy")?.to_string();
    let dir = test_copy(&name)?;
    prism::dupes::check_before_launch(&dir)?;
    let owner = label(need(args, "owner")?, "owner")?;
    let purpose = label(need(args, "purpose")?, "purpose")?;
    let eta = num(args, "eta", 5, 1, 30)?;
    let timeout = num(args, "timeout", 900, 60, 3600)?;
    // Always in the background (user, 2026-10-24): "park": false is no longer honoured.
    let park = true;
    let mut extra = jvm_props(args)?;
    // camera {pitch, yaw}: the harness holds the player's view there, for aimed screenshots.
    if let Some(cam) = args.get("camera") {
        let deg = |k: &str, lo: f64, hi: f64| -> Result<Option<f64>, String> {
            match cam.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v.as_f64().filter(|d| (lo..=hi).contains(d)).map(Some)
                    .ok_or(format!("camera.{k} must be a number from {lo} to {hi}")),
            }
        };
        if let Some(p) = deg("pitch", -90.0, 90.0)? { extra.push(format!("-Dtestharness.pitch={p}")); }
        if let Some(y) = deg("yaw", -360.0, 360.0)? { extra.push(format!("-Dtestharness.yaw={y}")); }
    }
    // test_world: a custom world built for this run (flat layers, border, platform, scripted events), then opened.
    let built;
    let args_world = match args.get("test_world").filter(|v| !v.is_null()) {
        Some(spec) => {
            if s(args, "world").is_some() || s(args, "new_world").is_some() {
                return Err("give test_world, world or new_world, not more than one".into());
            }
            if game_running_in(&dir) {
                return Err(format!("a game is already running in {name}"));
            }
            let (save, hold) = prism::testworld::build(&dir, "1.21.1", spec, now())?;
            built = save;
            // The autobench stays in the world this long, so the events get to run.
            if !extra.iter().any(|a| a.starts_with("-Dspeedloader.hold=") || a.starts_with("-Doptimal.hold=")) {
                extra.push(format!("-Dspeedloader.hold={hold}"));
            }
            Some(built.as_str())
        }
        None => s(args, "world"),
    };
    // world "new:<seed>" is the same as new_world (Speed Loader's own spelling).
    let (world, seed) = match args_world.map(|w| w.strip_prefix("new:").ok_or(w)) {
        Some(Ok(seed)) => (None, Some(seed)),
        Some(Err(w)) => (Some(w), s(args, "new_world")),
        None => (None, s(args, "new_world")),
    };
    if let Some(w) = world {
        plain_name(w, "world").map_err(|_| "world must be a plain save folder name".to_string())?;
        if !dir.join("saves").join(w).is_dir() {
            return Err(format!("no save called {w} in {name}"));
        }
        extra.push(format!("-Dspeedloader.autobench={w}"));
    }
    let new_world = match seed {
        Some(_) if world.is_some() => return Err("give either world or new_world, not both".into()),
        Some(seed) => {
            let ok = !seed.is_empty() && seed.len() <= 32 && seed.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
            if !ok { return Err("new_world must be a seed of letters, digits or -".into()); }
            extra.push(format!("-Dspeedloader.autobench=new:{seed}"));
            // Left over by a run that was killed before its cleanup (Speed Loader names them slnew-<label>).
            for n in save_names(&dir).into_iter().filter(|n| n.to_string_lossy().starts_with("slnew-")) {
                let _ = std::fs::remove_dir_all(dir.join("saves").join(n));
            }
            true
        }
        None => false,
    };
    if args.get("jfr").and_then(|v| v.as_bool()).unwrap_or(false) {
        let jfr = dir.join("jfr");
        let _ = std::fs::create_dir_all(&jfr);
        // jfr_roots: old-object samples keep their reference chain to a GC root (who holds them), found when the recording is written.
        let roots = if args.get("jfr_roots").and_then(|v| v.as_bool()).unwrap_or(false) { ",path-to-gc-roots=true" } else { "" };
        extra.push(format!("-XX:StartFlightRecording=name=lightning,settings=profile,class-loading=true{roots},filename={}/obelisk-{}.jfr,dumponexit=true",
            jfr.to_string_lossy().replace('\\', "/"), now()));
    }
    let swap = ModSwap::plan(&dir, args)?;
    let hooks = plan_hooks(args)?;
    if game_running_in(&dir) {
        return Err(format!("a game is already running in {name}"));
    }
    // Build the command now so a broken copy fails before queueing.
    let (mut java, mut cmd) = game_command(&dir, &extra)?;
    // The pack's own Java and JVM settings (its profile in Prism), unless the run asks for a bench Java.
    let profile = copy_source(&name);
    match args.get("java").filter(|v| !v.is_null()) {
        None => if let Some(p) = profile.java.as_ref() { java = p.clone() },
        Some(_) => match num(args, "java", 21, 21, 25)? {
            21 => {}
            25 => java = jdk25()?,
            _ => return Err("java must be 21 or 25".into()),
        },
    }
    profile.apply(&mut cmd);
    if prism::java::at(Path::new(&java)).is_some_and(|j| j.major >= 24) {
        cmd.retain(|a| a != "-XX:+ZGenerational"); // ZGC is generational-only from Java 24
    }
    if args.get("compact_headers").and_then(|v| v.as_bool()).unwrap_or(false) {
        if !java.contains("jdk-25") { return Err("compact_headers needs java 25".into()); }
        cmd.insert(0, "-XX:+UseCompactObjectHeaders".into());
    }
    let lock = format!("{owner} - {purpose} (obelisk)");
    let id = new_job(jobs, "launch", &name, &lock, dir.join("logs").join("latest.log").to_string_lossy().into_owned());
    let jobs = jobs.clone();
    // Only a performance test needs the bench to itself; anything else shares it, one run per copy.
    let perf = args.get("perf").and_then(|v| v.as_bool()).unwrap_or(false);
    std::thread::spawn(move || run_game(jobs, id, dir, lock, eta, timeout, park, perf, java, cmd, swap, new_world, hooks));
    Ok(json!({ "job": id, "state": "queued", "lock": format!("waiting for the bench lock as \"{owner} - {purpose} (obelisk)\"") }))
}

#[allow(clippy::too_many_arguments)]
fn run_game(jobs: Shared, id: u32, dir: PathBuf, lock: String, eta: u64, timeout: u64, park: bool, perf: bool, java: String, cmd: Vec<String>,
    mut swap: ModSwap, new_world: bool, mut hooks: Vec<Hook>) {
    let me = std::process::id().to_string();
    let copy = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let eta_s = eta.to_string();
    let mut acquire = vec!["acquire", &lock, "--pid", &me, "--wait", "--eta", &eta_s];
    if !perf { acquire.extend(["--shared", "--copy", &copy]); }
    let child = bench(&acquire).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    let Ok(mut child) = child else {
        update(&jobs, id, |j| { j.state = "failed".into(); j.detail = "couldn't run benchlock.py".into(); j.ended = Some(now()); });
        return;
    };
    update(&jobs, id, |j| j.helper_pid = Some(child.id()));
    // While queued, say who holds the lock and until when, so a wait shows why it's slow.
    let got = loop {
        if let Ok(Some(s)) = child.try_wait() { break s.code() == Some(0); }
        let holder = bench(&["status"]).output().ok().and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok());
        let d = match holder {
            Some(h) if h["owner"].is_string() => Some(format!("waiting for the bench lock: {} has it, expected to end {}",
                h["owner"].as_str().unwrap_or("?"), h["expected_end"].as_str().unwrap_or("?"))),
            Some(h) if h["shared"].is_array() => Some(format!("waiting for the bench lock: shared by {}",
                h["shared"].as_array().into_iter().flatten().map(|s| format!("{} in {} (until {})", s["owner"].as_str().unwrap_or("?"),
                    s["copy"].as_str().unwrap_or("?"), s["expected_end"].as_str().unwrap_or("?"))).collect::<Vec<_>>().join(", "))),
            _ => None,
        };
        if let Some(d) = d { update(&jobs, id, |j| if j.detail != KILLED { j.detail = d }); }
        std::thread::sleep(Duration::from_secs(15));
    };
    update(&jobs, id, |j| if j.detail.starts_with("waiting for the bench lock") { j.detail.clear() });
    update(&jobs, id, |j| j.helper_pid = None);
    if !got {
        finish(&jobs, id, "failed", "didn't get the bench lock".into());
        return;
    }
    let saves_before = if new_world { save_names(&dir) } else { Default::default() };
    let result = (|| -> Result<String, String> {
        swap.apply()?;
        // Test rules: harness jar (no mouse grab, no focus), muted, keeps running unfocused.
        let has_harness = std::fs::read_dir(dir.join("mods")).into_iter().flatten().flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("testharness"));
        // Kept current: an older harness in the copy lacks newer switches (camera aim).
        let harness = Path::new(&tb().harness);
        let jar = dir.join("mods").join(harness.file_name().ok_or("the test bench names no harness jar")?);
        let stale = std::fs::read(&jar).ok() != std::fs::read(harness).ok();
        if !has_harness || stale {
            for e in std::fs::read_dir(dir.join("mods")).into_iter().flatten().flatten() {
                if e.file_name().to_string_lossy().starts_with("testharness") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
            std::fs::copy(harness, &jar).map_err(|e| format!("test harness jar: {e}"))?;
        }
        set_option(&dir.join("options.txt"), "soundCategory_master", "0.0");
        test_rules(&dir);
        // Run logs live beside the copies, never inside one: a copy's folder must match the pack it came from.
        let runs = tb().root.join("obelisk-runs").join(dir.file_name().unwrap_or_default());
        let _ = std::fs::create_dir_all(&runs);
        let out = std::fs::File::create(runs.join(format!("{id}.stdout"))).map_err(|e| e.to_string())?;
        // The exact Java and arguments, for when a run dies before the game writes any log.
        let _ = std::fs::write(runs.join(format!("{id}.cmd.txt")), format!("{java}\n{}\n", cmd.join("\n")));
        let started = SystemTime::now();
        let click_ms = prism::timeline::now_ms();
        let mut game = Command::new(&java).args(&cmd).current_dir(&dir).stdout(out.try_clone().map_err(|e| e.to_string())?).stderr(out)
            .creation_flags_no_window().spawn().map_err(|e| format!("couldn't start Java: {e}"))?;
        let pid = game.id();
        update(&jobs, id, |j| { j.state = "running".into(); j.pid = Some(pid); j.java = Some(java.clone()); });
        let t0 = Instant::now();
        let mut parked = std::collections::HashSet::new();
        let mut stopping: Option<Instant> = None;
        let mut window_ms = None;
        let mut watch = super::devrun::Sampler::new(pid, &dir, &runs, id, click_ms, super::devrun::jcmd_for(&java).ok());
        let mut shown = Instant::now();
        let latest = dir.join("logs").join("latest.log");
        let why = loop {
            if let Ok(Some(status)) = game.try_wait() {
                let _ = std::fs::write(runs.join(format!("{id}.exit")), status.code().map(|c| c.to_string()).unwrap_or("?".into()));
                break format!("exited ({})", status.code().map(|c| c.to_string()).unwrap_or("?".into()));
            }
            if park {
                park_windows(pid, &mut parked);
            }
            if window_ms.is_none() && super::top_windows().iter().any(|w| w.1 == pid && w.2) {
                let ms = prism::timeline::now_ms();
                window_ms = Some(ms);
                watch.add("window", ms, "obelisk");
            }
            let since = (prism::timeline::now_ms() - click_ms) as f64 / 1000.0;
            let mut kill_now = None;
            for h in hooks.iter_mut().filter(|h| !h.fired) {
                let Some(at) = watch.events.iter().find(|e| e["event"] == h.event.as_str()).and_then(|e| e["t"].as_f64()) else { continue };
                if since < at + h.after { continue; }
                h.fired = true;
                let base = json!({ "event": h.event, "after": h.after, "do": h.action, "t": (since * 10.0).round() / 10.0 });
                if h.action == "kill" {
                    kill_now = Some(format!("ended by \"at\" {} +{}s", h.event, h.after));
                    update(&jobs, id, |j| j.hooks.push(base));
                    continue;
                }
                // Heap snapshots take seconds (a dump much longer): off the watch loop.
                let (jobs, java, out, kind) = (jobs.clone(), java.clone(), dir.join("jfr"), h.action.clone());
                std::thread::spawn(move || {
                    let r = super::devrun::jcmd_for(&java).and_then(|j| snapshot(&j, pid, &out, &kind));
                    let mut v = base;
                    match r { Ok(r) => v["result"] = r, Err(e) => v["error"] = e.into() }
                    update(&jobs, id, |j| j.hooks.push(v));
                });
            }
            if let Some(why) = kill_now {
                super::kill_tree(pid);
                let _ = game.wait();
                break why;
            }
            // New stages wake a wait {until}; samples reach the job every 5 s.
            if watch.tick() || shown.elapsed() >= Duration::from_secs(5) {
                shown = Instant::now();
                let (events, now) = (watch.events.clone(), watch.latest.clone());
                update(&jobs, id, |j| { j.events = events; j.now = now; });
            }
            if stopping.is_none() && log_says_stopping(&latest, started) {
                stopping = Some(Instant::now());
            }
            let hung_on_exit = stopping.is_some_and(|t| t.elapsed() > Duration::from_secs(30));
            if hung_on_exit || t0.elapsed() > Duration::from_secs(timeout) {
                super::kill_tree(pid);
                let _ = game.wait();
                break if hung_on_exit { "killed: still alive 30 s after Stopping!".into() } else { format!("killed: ran past {timeout} s") };
            }
            std::thread::sleep(Duration::from_millis(250));
        };
        let t = prism::timeline::record(&dir, prism::timeline::Session { source: "test".into(), label: lock.clone(), click_ms,
            java_ms: Some(click_ms), window_ms, exit_ms: Some(prism::timeline::now_ms()), ..Default::default() });
        let times: Vec<String> = prism::timeline::phases(&t).iter().filter(|p| !p.idle && p.key != "launcher")
            .map(|p| format!("{} {:.1}s", p.key, p.ms as f64 / 1000.0)).collect();
        let why = if times.is_empty() { why } else { format!("{why}; {}", times.join(", ")) };
        let crashes = std::fs::read_dir(dir.join("crash-reports")).into_iter().flatten().flatten()
            .filter(|e| e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t >= started)).count();
        Ok(if crashes > 0 { format!("{why}; {crashes} new crash report(s)") } else { why })
    })();
    let restored = swap.restore();
    if new_world {
        for n in save_names(&dir).difference(&saves_before) {
            let _ = std::fs::remove_dir_all(dir.join("saves").join(n));
        }
    }
    let result = match (result, restored) {
        (Ok(d), Err(e)) => Ok(format!("{d}; {e}")),
        (r, _) => r,
    };
    let _ = bench(&["release", &lock]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    let (state, detail) = match result {
        Ok(d) if d.starts_with("killed") || d.contains("crash report") => ("failed", d),
        Ok(d) => ("done", d),
        Err(e) => ("failed", e),
    };
    audit("launch-end", &json!({ "job": id }), &format!("{state}: {detail}"));
    finish(&jobs, id, state, detail);
}

/// What a copy's game runs with from the profile it was copied from: its Java and its JVM settings.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Profile {
    java: Option<String>,
    args: Vec<String>,
    max_mem_mb: Option<u32>,
    min_mem_mb: Option<u32>,
    /// Library overrides from the profile's own Prism components (patches/<uid>.json), e.g. a newer Mixin.
    #[serde(default)]
    libraries: Vec<Value>,
}

impl Profile {
    /// Read from a Prism profile's instance.cfg (only the settings it overrides).
    fn of_instance(inst: &Path) -> Profile {
        let cfg = prism::read_cfg(&inst.join("instance.cfg"));
        let on = |k: &str| cfg.get(k).is_some_and(|v| v == "true");
        let num = |k: &str| cfg.get(k).and_then(|v| v.parse().ok());
        Profile {
            java: cfg.get("JavaPath").filter(|_| on("OverrideJavaLocation")).map(|p| prism::java::resolve(&prism::data_dir(), p))
                .filter(|p| p.is_file()).map(|p| p.to_string_lossy().replace('\\', "/")),
            args: if on("OverrideJavaArgs") { cfg.get("JvmArgs").map(|a| a.split_whitespace().map(String::from).collect()).unwrap_or_default() } else { vec![] },
            max_mem_mb: if on("OverrideMemory") { num("MaxMemAlloc") } else { None },
            min_mem_mb: if on("OverrideMemory") { num("MinMemAlloc") } else { None },
            libraries: profile_libraries(inst),
        }
    }

    /// Put the profile's memory and arguments in place of the bench defaults (one garbage collector only).
    fn apply(&self, cmd: &mut Vec<String>) {
        if let Some(mx) = self.max_mem_mb { cmd.retain(|a| !a.starts_with("-Xmx")); cmd.insert(0, format!("-Xmx{mx}m")); }
        if let Some(ms) = self.min_mem_mb { cmd.retain(|a| !a.starts_with("-Xms")); cmd.insert(0, format!("-Xms{ms}m")); }
        if self.args.iter().any(|a| a.starts_with("-XX:+Use") && a.ends_with("GC")) {
            cmd.retain(|a| !(a.starts_with("-XX:+Use") && a.ends_with("GC")) && a != "-XX:+ZGenerational");
        }
        for (i, a) in self.args.iter().enumerate() { cmd.insert(i, a.clone()) }
    }
}

/// The libraries a profile's custom components swap in: those in its mmc-pack.json with a patches/<uid>.json
/// that only swaps libraries (the same check a collab applies to one from another PC).
fn profile_libraries(inst: &Path) -> Vec<Value> {
    let pack: Value = std::fs::read(inst.join("mmc-pack.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut out = vec![];
    for uid in pack["components"].as_array().into_iter().flatten().filter_map(|c| c["uid"].as_str()) {
        if !uid.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) { continue }
        let Ok(bytes) = std::fs::read(inst.join("patches").join(format!("{uid}.json"))) else { continue };
        if prism::share::library_patch_ok(&bytes).is_err() { continue }
        let v: Value = serde_json::from_slice(&bytes).unwrap_or_default();
        out.extend(v["libraries"].as_array().cloned().unwrap_or_default());
    }
    out
}

/// Swap the profile's library overrides into a classpath, in place of the same group:artifact. The jar comes from
/// the install's libraries, else Prism's, else its Maven download, and its checksum must match either way.
fn override_libraries(cp: &mut Vec<String>, libs: &[Value], lib: &str) -> Result<(), String> {
    for l in libs {
        let name = l["name"].as_str().unwrap_or("");
        let parts: Vec<&str> = name.split(':').collect();
        let [g, a, v, ..] = parts[..] else { continue };
        let g = g.replace('.', "/");
        let rel = format!("{g}/{a}/{v}/{a}-{v}.jar");
        let sha1 = l.pointer("/downloads/artifact/sha1").and_then(|s| s.as_str()).unwrap_or("").to_ascii_lowercase();
        let dest = PathBuf::from(format!("{lib}/{rel}"));
        let good = |p: &Path| std::fs::read(p).is_ok_and(|b| prism::sha1_hex(&b) == sha1);
        if !good(&dest) {
            let prism_copy = prism::data_dir().join("libraries").join(&rel);
            let bytes = if good(&prism_copy) {
                std::fs::read(&prism_copy).map_err(|e| e.to_string())?
            } else {
                let url = l.pointer("/downloads/artifact/url").and_then(|u| u.as_str()).ok_or_else(|| format!("{name}: no download"))?;
                let b = prism::modrinth::Client::new()?.fetch(url)?;
                if prism::sha1_hex(&b) != sha1 { return Err(format!("{name}: the download doesn't match its checksum")) }
                b
            };
            std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&dest, bytes).map_err(|e| e.to_string())?;
        }
        let prefix = format!("{lib}/{g}/{a}/");
        let at = cp.iter().position(|p| p.starts_with(&prefix)).unwrap_or(cp.len());
        cp.retain(|p| !p.starts_with(&prefix));
        cp.insert(at.min(cp.len()), format!("{lib}/{rel}"));
    }
    Ok(())
}

/// The newest bench run of this copy, when it exited with an error: its code and the JVM's output.
fn failed_run(dir: &Path) -> Option<(String, Vec<String>)> {
    let runs = tb().root.join("obelisk-runs").join(dir.file_name()?);
    let newest = std::fs::read_dir(&runs).ok()?.flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "stdout"))
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())?;
    let code = std::fs::read_to_string(newest.with_extension("exit")).ok()?.trim().to_string();
    if code == "0" { return None }
    let text = String::from_utf8_lossy(&std::fs::read(&newest).ok()?).into_owned();
    Some((code, text.lines().map(String::from).collect()))
}

/// Plain words for Java errors that need no stack trace to understand.
fn explain_java_error(line: &str) -> Option<String> {
    let v = line.split("class file version ").nth(1)?.split(['.', ')']).next()?.parse::<u32>().ok()?;
    let has = line.split("up to ").nth(1).and_then(|r| r.split('.').next()).and_then(|n| n.parse::<u32>().ok());
    Some(format!("a mod needs Java {} but the game ran on Java {} (set the profile's Java, or launch with java: 25)",
        v.saturating_sub(44), has.map(|h| h.saturating_sub(44).to_string()).unwrap_or("?".into())))
}

fn source_file(copy: &str) -> PathBuf {
    tb().root.join("obelisk-runs").join(copy).join("source.json")
}

/// The profile a copy came from (recorded by copy_pack). Copies made before that was recorded are taken to come
/// from the bench's play profile, which is where copies are made from.
fn copy_source(copy: &str) -> Profile {
    std::fs::read(source_file(copy)).ok().and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| play_dir().parent().map(Profile::of_instance).unwrap_or_default())
}

/// Java 25 for A/B runs: the newest <bench>\java\jdk-25* folder.
fn jdk25() -> Result<String, String> {
    let mut homes: Vec<PathBuf> = std::fs::read_dir(tb().root.as_path().join("java")).into_iter().flatten().flatten()
        .map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("jdk-25"))).collect();
    homes.sort();
    let exe = homes.last().map(|h| h.join("bin").join("java.exe")).filter(|p| p.is_file());
    exe.map(|p| p.to_string_lossy().replace('\\', "/")).ok_or_else(|| "no Java 25 in the test bench's java folder (jdk-25*)".into())
}

fn save_names(dir: &Path) -> std::collections::HashSet<std::ffi::OsString> {
    std::fs::read_dir(dir.join("saves")).into_iter().flatten().flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name()).collect()
}

/// Per-run changes: jars in mods/ set aside, jars from the test bench added and TOML config keys set, all undone after the game exits.
#[derive(Default)]
struct ModSwap {
    mods: PathBuf,
    disable: Vec<String>,
    add: Vec<PathBuf>,
    off: Vec<PathBuf>,
    added: Vec<PathBuf>,
    /// Config file, its original bytes, the edited text.
    configs: Vec<(PathBuf, Vec<u8>, String)>,
    written: Vec<usize>,
}

impl ModSwap {
    fn plan(dir: &Path, args: &Value) -> Result<Self, String> {
        let list = |key: &str| -> Result<Vec<String>, String> {
            match args.get(key) {
                None | Some(Value::Null) => Ok(vec![]),
                Some(v) => v.as_array().ok_or(format!("\"{key}\" must be a list"))?.iter()
                    .map(|x| x.as_str().map(String::from).ok_or(format!("\"{key}\" entries must be strings"))).collect(),
            }
        };
        let mods = dir.join("mods");
        let disable = list("disable_mods")?;
        for f in &disable {
            let plain = !f.contains(['/', '\\']) && !f.starts_with('.') && f.to_ascii_lowercase().ends_with(".jar");
            if !plain || !mods.join(f).is_file() { return Err(format!("disable_mods: no {f} in mods/")); }
        }
        let add = list("add_mods")?.iter().map(|p| mcw_jar(p)).collect::<Result<Vec<_>, _>>()?;
        for a in &add {
            if mods.join(a.file_name().unwrap_or_default()).exists() {
                return Err(format!("add_mods: mods/ already has {}", a.file_name().unwrap_or_default().to_string_lossy()));
            }
        }
        // config: [{file, key, value}], plain TOML; every key must already exist.
        let mut configs: Vec<(PathBuf, Vec<u8>, String)> = vec![];
        let edits = match args.get("config") {
            None | Some(Value::Null) => vec![],
            Some(v) => v.as_array().ok_or("\"config\" must be a list")?.clone(),
        };
        for e in &edits {
            let field = |k: &str| e.get(k).and_then(|v| v.as_str()).ok_or(format!("config entries need \"{k}\" as text"));
            let (file, key, value) = (field("file")?, field("key")?, field("value")?);
            let rel = Path::new(file);
            let inside = rel.components().all(|c| matches!(c, std::path::Component::Normal(_)));
            if !inside || !file.to_ascii_lowercase().ends_with(".toml") {
                return Err(format!("config: {file} must be a .toml path inside the copy"));
            }
            let path = dir.join(rel);
            let i = match configs.iter().position(|c| c.0 == path) {
                Some(i) => i,
                None => {
                    let bytes = std::fs::read(&path).map_err(|e| format!("config: {file}: {e}"))?;
                    let text = String::from_utf8(bytes.clone()).map_err(|_| format!("config: {file} isn't UTF-8"))?;
                    configs.push((path, bytes, text));
                    configs.len() - 1
                }
            };
            configs[i].2 = prism::tomlset::set_key(&configs[i].2, key, value).map_err(|e| format!("config: {file}: {e}"))?;
        }
        Ok(ModSwap { mods, disable, add, configs, ..Default::default() })
    }

    fn apply(&mut self) -> Result<(), String> {
        for f in &self.disable {
            let from = self.mods.join(f);
            let to = self.mods.join(format!("{f}.obelisk-off"));
            std::fs::rename(&from, &to).map_err(|e| format!("couldn't set aside {f}: {e}"))?;
            self.off.push(to);
        }
        for a in &self.add {
            let to = self.mods.join(a.file_name().unwrap_or_default());
            std::fs::copy(a, &to).map_err(|e| format!("couldn't add {}: {e}", a.display()))?;
            self.added.push(to);
        }
        for (i, (path, _, text)) in self.configs.iter().enumerate() {
            std::fs::write(path, text).map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
            self.written.push(i);
        }
        Ok(())
    }

    /// Undo everything apply did, even after a failed run.
    fn restore(&mut self) -> Result<(), String> {
        let mut bad = vec![];
        for p in self.added.drain(..) {
            if std::fs::remove_file(&p).is_err() { bad.push(p); }
        }
        for p in self.off.drain(..) {
            let back = p.with_extension("");
            if std::fs::rename(&p, &back).is_err() { bad.push(p); }
        }
        for i in self.written.drain(..) {
            let (path, bytes, _) = &self.configs[i];
            if std::fs::write(path, bytes).is_err() { bad.push(path.clone()); }
        }
        if bad.is_empty() { Ok(()) } else { Err(format!("couldn't restore: {bad:?}")) }
    }
}

fn log_says_stopping(latest: &Path, since: SystemTime) -> bool {
    let Ok(meta) = std::fs::metadata(latest) else { return false };
    if meta.modified().map(|t| t < since).unwrap_or(true) {
        return false;
    }
    let Ok(mut f) = std::fs::File::open(latest) else { return false };
    use std::io::Seek;
    let len = meta.len();
    let _ = f.seek(std::io::SeekFrom::Start(len.saturating_sub(16 * 1024)));
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).contains("Stopping!")
}

/// Beyond every monitor, at the bottom of the z-order, never activated (as gate.py does).
fn park_windows(pid: u32, parked: &mut std::collections::HashSet<isize>) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetWindowPos(h: isize, after: isize, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    }
    for (h, p, vis, _, _) in super::top_windows() {
        if p == pid && vis && parked.insert(h) {
            unsafe { SetWindowPos(h, 1, -20000, -20000, 0, 0, 0x0001 | 0x0010) };
        }
    }
}

// ---------------------------------------------------------------- gate and copies

fn gate(args: &Value, jobs: &Shared) -> Result<Value, String> {
    let owner = label(need(args, "owner")?, "owner")?;
    let mut a: Vec<String> = vec![format!("{}/gate.py", tb().scripts), "run".into(), "--owner".into(), owner.clone()];
    a.extend(["--cycles".into(), num(args, "cycles", 2, 1, 5)?.to_string(), "--hold".into(), num(args, "hold", 120, 30, 600)?.to_string()]);
    if args.get("sync").and_then(|v| v.as_bool()).unwrap_or(true) {
        a.push("--sync".into());
    }
    if let Some(jar) = s(args, "jar") {
        let jar = mcw_jar(jar)?;
        let replace = need(args, "replace").map_err(|_| "with \"jar\", give \"replace\": the glob of the jar(s) it replaces in mods/")?;
        if !replace.chars().all(|c| c.is_ascii_alphanumeric() || "*._-+".contains(c)) {
            return Err("\"replace\" must be a simple file glob like boundless-*.jar".into());
        }
        a.extend(["--swap".into(), jar.to_string_lossy().trim_start_matches(r"\\?\").to_string(), "--replace".into(), replace.into()]);
    }
    if let Some(m) = s(args, "mods") {
        if !m.chars().all(|c| c.is_ascii_alphanumeric() || ",_-.".contains(c)) {
            return Err("\"mods\" is a comma-separated list of mod ids".into());
        }
        a.extend(["--mods".into(), m.into()]);
    }
    if let Some(t) = s(args, "tag") {
        plain_name(t, "tag")?;
        a.extend(["--tag".into(), t.into()]);
    }
    if let Some(v) = s(args, "loader_version") {
        if !v.starts_with("neoforge-") || !v.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c)) {
            return Err("\"loader_version\" is a neoforge-... version folder name in the install".into());
        }
        a.extend(["--loader-version".into(), v.into()]);
    }
    let out = tb().root.as_path().join("obelisk").join("jobs");
    let id = new_job(jobs, "gate", "gate", &owner, String::new());
    let out = out.join(format!("{id}-gate.txt"));
    update(jobs, id, |j| j.output = out.to_string_lossy().into_owned());
    let jobs = jobs.clone();
    std::thread::spawn(move || {
        let r = (|| -> Result<String, String> {
            let f = std::fs::File::create(&out).map_err(|e| e.to_string())?;
            let mut child = python().args(&a).current_dir(&tb().root).stdout(f.try_clone().map_err(|e| e.to_string())?).stderr(f)
                .spawn().map_err(|e| e.to_string())?;
            update(&jobs, id, |j| { j.state = "running".into(); j.pid = Some(child.id()); });
            let code = child.wait().map_err(|e| e.to_string())?.code();
            let text = std::fs::read_to_string(&out).unwrap_or_default();
            let verdict = if text.contains("PASS") && code == Some(0) { "PASS" } else { "FAIL" };
            Ok(format!("{verdict} (exit {})", code.map(|c| c.to_string()).unwrap_or("killed".into())))
        })();
        let (state, detail) = match r { Ok(d) if d.starts_with("PASS") => ("done", d), Ok(d) => ("failed", d), Err(e) => ("failed", e) };
        audit("gate-end", &json!({ "job": id }), &format!("{state}: {detail}"));
        finish(&jobs, id, state, detail);
    });
    Ok(json!({ "job": id, "state": "queued", "output": "see status/wait for the summary file" }))
}

fn copy_pack(args: &Value, jobs: &Shared) -> Result<Value, String> {
    let from = readable(need(args, "from")?)?;
    let to_name = need(args, "to")?.to_string();
    plain_name(&to_name, "to")?;
    if NOT_COPIES.contains(&to_name.to_ascii_lowercase().as_str()) {
        return Err(format!("{to_name} is reserved"));
    }
    let to = tb().root.as_path().join(&to_name);
    // Only into a new or empty folder, so nobody's copy is ever overwritten.
    if to.exists() && std::fs::read_dir(&to).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Err(format!("{to_name} already exists in the test bench; pick a new name"));
    }
    let saves = args.get("saves").and_then(|v| v.as_bool()).unwrap_or(false);
    let owner = label(s(args, "owner").unwrap_or("unknown"), "owner")?;
    let id = new_job(jobs, "copy-pack", &to_name, &owner, to.to_string_lossy().into_owned());
    let jobs = jobs.clone();
    std::thread::spawn(move || {
        update(&jobs, id, |j| j.state = "running".into());
        let mut c = Command::new("robocopy");
        c.arg(&from).arg(&to).args(["/E", "/R:1", "/W:1", "/NFL", "/NDL", "/NJH", "/NP", "/XJ", "/XD"])
            .args(COPY_SKIP_DIRS.iter().filter(|d| saves || **d != "saves")).arg("mods-removed*")
            .args(["/XF", "*.log", "session.lock", ".stignore", "gate-sync.json"])
            .creation_flags_no_window().stdout(Stdio::null()).stderr(Stdio::null());
        let code = c.status().ok().and_then(|s| s.code()).unwrap_or(16);
        // robocopy: below 8 means success.
        let (state, detail) = if code < 8 { ("done", format!("copied to {}", to.display())) } else { ("failed", format!("robocopy exit {code}")) };
        if code < 8 {
            test_rules(&to);
            // Files Obelisk blocked in the pack stay out of the copy too.
            for h in prism::held::list(&from) {
                if !h.path.contains("..") { let _ = std::fs::remove_file(to.join(&h.path)); }
            }
            let profile = if from == play_dir() { from.parent().map(Profile::of_instance).unwrap_or_default() }
                else { copy_source(&from.file_name().unwrap_or_default().to_string_lossy()) };
            let _ = std::fs::create_dir_all(tb().root.join("obelisk-runs").join(&to_name));
            let _ = std::fs::write(source_file(&to_name), serde_json::to_vec_pretty(&profile).unwrap_or_default());
        }
        audit("copy-pack-end", &json!({ "job": id }), &format!("{state}: {detail}"));
        update(&jobs, id, |j| { j.state = state.into(); j.detail = detail; j.ended = Some(now()); });
    });
    Ok(json!({ "job": id, "state": "running" }))
}
