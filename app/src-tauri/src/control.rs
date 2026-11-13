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
/// The pack test copies are made from unless said otherwise (Test Dummy is the only target, user 2026-11-05).
const TARGET_PACK: &str = "Test Dummy";
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
    /// Launches: screenshots Windows took of the game's window when it stalled ({t, why, path, note}).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shots: Vec<Value>,
}

#[derive(Default)]
pub(crate) struct Jobs {
    next: u32,
    list: HashMap<u32, Job>,
}

pub(crate) type Shared = Arc<(Mutex<Jobs>, Condvar)>;

fn now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn update(jobs: &Shared, id: u32, f: impl FnOnce(&mut Job)) {
    let (m, cv) = &**jobs;
    if let Some(j) = m.lock().unwrap().list.get_mut(&id) {
        f(j);
        journal::state(id, &j.state);
    }
    cv.notify_all();
}

/// Jobs across an Obelisk restart (an update restarts it, often while chats have launches queued): ids keep counting
/// up instead of starting again at 1, a launch or gate still waiting for the bench lock is submitted again under its
/// own id, and one that was running answers "lost on restart" instead of "no such job" (Patchwork, 2026-11-06).
mod journal {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Serialize, Deserialize, Default)]
    pub struct Journal {
        pub next: u32,
        /// Jobs not finished yet: their state, and (launch, gate) the request to submit again.
        pub open: BTreeMap<u32, Open>,
        /// Jobs a restart ended: id → why (the newest 200).
        pub lost: BTreeMap<u32, String>,
    }

    #[derive(Serialize, Deserialize, Clone)]
    pub struct Open {
        pub state: String,
        pub tool: Option<String>,
        pub args: Option<Value>,
    }

    static J: Mutex<Option<Journal>> = Mutex::new(None);
    thread_local! {
        /// The id new_job uses for a job submitted again after a restart.
        pub static REUSE: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
    }

    fn path() -> std::path::PathBuf { super::tb().root.join("obelisk").join("jobs").join("journal.json") }

    fn save(j: &Journal) {
        let p = path();
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec(j).unwrap_or_default()).is_ok() { let _ = std::fs::rename(&tmp, &p); }
    }

    fn with<T>(f: impl FnOnce(&mut Journal) -> T) -> Option<T> {
        let mut g = J.lock().ok()?;
        let j = g.as_mut()?;
        let out = f(j);
        save(j);
        Some(out)
    }

    /// At start-up: the last run's journal, with its open jobs taken out. Running ones become lost; the waiting ones
    /// with a request are returned in id order, to be submitted again.
    pub fn load() -> (u32, Vec<(u32, String, Value)>) {
        let mut j: Journal = std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let when = chrono_like_now();
        let mut again = vec![];
        for (id, o) in std::mem::take(&mut j.open) {
            match (o.state.as_str(), o.tool, o.args) {
                ("queued", Some(tool), Some(args)) => again.push((id, tool, args)),
                (st, _, _) => { j.lost.insert(id, format!("job {id} was {st} when Obelisk restarted at {when}, so it ended; submit it again")); }
            }
        }
        while j.lost.len() > 200 { let first = *j.lost.keys().next().unwrap(); j.lost.remove(&first); }
        let next = j.next;
        save(&j);
        *J.lock().unwrap() = Some(j);
        (next, again)
    }

    /// The local time of day, as the user's clock shows it.
    fn chrono_like_now() -> String {
        #[repr(C)]
        #[derive(Default)]
        struct SystemTime { y: u16, mo: u16, dow: u16, d: u16, h: u16, mi: u16, s: u16, ms: u16 }
        #[link(name = "kernel32")]
        unsafe extern "system" { fn GetLocalTime(t: *mut SystemTime); }
        let mut t = SystemTime::default();
        unsafe { GetLocalTime(&mut t) };
        format!("{:02}:{:02}", t.h, t.mi)
    }

    pub fn created(id: u32) {
        with(|j| { j.next = j.next.max(id); j.open.insert(id, Open { state: "queued".into(), tool: None, args: None }); });
    }

    /// The request behind a launch or gate job, kept while it waits for the bench lock.
    pub fn request(id: u32, tool: &str, args: &Value) {
        with(|j| if let Some(o) = j.open.get_mut(&id) { o.tool = Some(tool.into()); o.args = Some(args.clone()); });
    }

    pub fn state(id: u32, state: &str) {
        with(|j| match state {
            "queued" | "running" => { if let Some(o) = j.open.get_mut(&id) { o.state = state.into() } }
            _ => { j.open.remove(&id); }
        });
    }

    pub fn lose(id: u32, why: String) {
        with(|j| { j.open.remove(&id); j.lost.insert(id, why); });
    }

    /// Why a job id isn't known: lost on a restart, or never was.
    pub fn gone(id: u32) -> String {
        J.lock().ok().and_then(|g| g.as_ref()?.lost.get(&id).cloned()).unwrap_or_else(|| "no such job".into())
    }
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
    std::thread::spawn(recover_swaps);
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
    if tb_ready().is_ok() {
        let (next, again) = journal::load();
        jobs.0.lock().unwrap().next = next;
        // Submitted again in their old order, so they wait for the bench lock about where they were.
        let (app, jobs) = (app.clone(), jobs.clone());
        std::thread::spawn(move || for (id, tool, args) in again {
            journal::REUSE.with(|r| r.set(Some(id)));
            let r = dispatch(&tool, &args, &jobs, &app);
            journal::REUSE.with(|r| r.set(None));
            match r {
                Ok(_) => audit(&tool, &args, &format!("job {id} submitted again after a restart")),
                Err(e) => {
                    audit(&tool, &args, &format!("job {id} couldn't be submitted again after a restart: {e}"));
                    journal::lose(id, format!("job {id} was waiting when Obelisk restarted, and submitting it again failed: {e}"));
                }
            }
        });
    }
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
    stream.write_all(&body)?;
    // End the answer on the socket itself: a handle a child process (benchlock, gate) inherited mid-request would
    // otherwise keep the connection open, and the caller would wait for an end that never comes.
    stream.shutdown(std::net::Shutdown::Write)
}

// ---------------------------------------------------------------- validation

/// new_world as text or a plain number (new_world: 4250 was silently dropped and the game sat on the title screen).
fn seed_arg(args: &Value) -> Option<String> {
    let v = args.get("new_world")?;
    v.as_str().map(str::to_string).or_else(|| v.as_u64().map(|n| n.to_string())).filter(|v| !v.is_empty())
}

fn s<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str()).filter(|v| !v.is_empty())
}

pub(crate) fn need<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
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
pub(crate) fn test_copy(name: &str) -> Result<PathBuf, String> {
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

/// A garbage-collector setting: which collector, or how one is tuned (not memory size, not anything else).
fn is_gc_flag(a: &str) -> bool {
    let Some(rest) = a.strip_prefix("-XX:") else { return false };
    let name = rest.trim_start_matches(['+', '-']).split('=').next().unwrap_or("");
    let exact = ["ConcGCThreads", "ParallelGCThreads", "MaxGCPauseMillis", "GCPauseIntervalMillis", "ParallelRefProcEnabled",
        "UseStringDeduplication", "SoftMaxHeapSize", "InitiatingHeapOccupancyPercent", "GCTimeRatio", "MaxTenuringThreshold",
        "UseDynamicNumberOfGCThreads", "UseAdaptiveSizePolicy",
        // Some G1 tuning (G1NewSizePercent, G1MaxNewSizePercent) is experimental and needs this first.
        "UnlockExperimentalVMOptions"];
    (name.starts_with("Use") && name.ends_with("GC")) || name.starts_with("G1") || name.starts_with("Shenandoah")
        || (name.starts_with('Z') && name.len() > 1) || name.starts_with("ClassUnloading") || exact.contains(&name)
}

/// Heap size cap for a GC A/B run: the bench's 6 GB memory budget.
const HEAP_CAP_MB: u64 = 6144;

/// -Xms/-Xmx and its size in megabytes (k, m, g or bytes; None if unreadable), or None if it isn't one.
fn heap_flag(a: &str) -> Option<(&'static str, Option<u64>)> {
    let (kind, v) = if let Some(v) = a.strip_prefix("-Xmx") { ("-Xmx", v) } else { ("-Xms", a.strip_prefix("-Xms")?) };
    let (n, unit) = v.split_at(v.find(|c: char| !c.is_ascii_digit()).unwrap_or(v.len()));
    let Ok(n) = n.parse::<u64>() else { return Some((kind, None)) };
    let mb = match unit.to_ascii_lowercase().as_str() { "" => Some(n >> 20), "k" => Some(n >> 10), "m" => Some(n), "g" => n.checked_mul(1024), _ => None };
    Some((kind, mb))
}

/// "gc": this run's garbage collector and its tuning, in place of the profile's and the bench's (for GC A/B runs).
/// Exactly one -XX:+Use...GC, then only GC settings; nothing is saved, the next run is back to the profile's.
fn gc_flags(args: &Value) -> Result<Option<Vec<String>>, String> {
    let Some(list) = args.get("gc").filter(|v| !v.is_null()) else { return Ok(None) };
    let list = list.as_array().ok_or("\"gc\" must be a list of -XX GC flags, e.g. [\"-XX:+UseG1GC\", \"-XX:MaxGCPauseMillis=50\"]")?;
    let mut out = vec![];
    let mut heap: HashMap<&str, u64> = HashMap::new();
    for v in list {
        let a = v.as_str().ok_or("\"gc\" entries must be strings")?;
        if let Some((kind, mb)) = heap_flag(a) {
            let mb = mb.ok_or_else(|| format!("{a}: give the size as e.g. 4096m or 4g"))?;
            if mb == 0 || mb > HEAP_CAP_MB { return Err(format!("{a}: heap sizes go up to {HEAP_CAP_MB}m (the bench's 6 GB budget)")) }
            if heap.insert(kind, mb).is_some() { return Err(format!("\"gc\" has {kind} twice")) }
            out.push(a.to_string());
            continue;
        }
        let ok = is_gc_flag(a) && a.len() <= 80 && a.chars().all(|c| c.is_ascii_alphanumeric() || ":+-=._".contains(c));
        if !ok {
            return Err(format!("{a}: not a GC flag (allowed: -XX:+Use...GC, G1*/Z*/Shenandoah*/ClassUnloading* settings, ConcGCThreads, MaxGCPauseMillis and the like, -Xms/-Xmx up to {HEAP_CAP_MB}m)"));
        }
        out.push(a.to_string());
    }
    let collectors = out.iter().filter(|a| a.starts_with("-XX:+Use") && a.ends_with("GC")).count();
    if collectors != 1 {
        return Err(format!("\"gc\" needs exactly one collector (-XX:+UseZGC, -XX:+UseG1GC, -XX:+UseShenandoahGC...), got {collectors}"));
    }
    if let (Some(ms), Some(mx)) = (heap.get("-Xms"), heap.get("-Xmx")) {
        if ms > mx { return Err(format!("-Xms ({ms}m) is larger than -Xmx ({mx}m)")) }
    }
    Ok(Some(out))
}

/// "diag": read-only JVM diagnostics for a test copy's run: safepoint and handshake timeout prints, and -Xlog of
/// safepoint/handshake tags to a file inside the copy (the game runs in the copy's folder, so a relative path lands there).
/// Nothing that changes behaviour: AbortVMOnSafepointTimeout and the like are refused.
fn diag_flags(args: &Value) -> Result<Vec<String>, String> {
    let Some(list) = args.get("diag").filter(|v| !v.is_null()) else { return Ok(vec![]) };
    let list = list.as_array().ok_or("\"diag\" must be a list of JVM diagnostic flags")?;
    let ms = |v: &str| v.parse::<u32>().is_ok_and(|n| (1..=600_000).contains(&n));
    let mut out = vec![];
    for v in list {
        let a = v.as_str().ok_or("\"diag\" entries must be strings")?;
        let ok = match a {
            "-XX:+UnlockDiagnosticVMOptions" | "-XX:+SafepointTimeout" => true,
            _ if a.starts_with("-XX:SafepointTimeoutDelay=") => ms(&a[26..]),
            _ if a.starts_with("-XX:HandshakeTimeout=") => ms(&a[21..]),
            _ => xlog_ok(a),
        };
        if !ok {
            return Err(format!("{a}: not an allowed diagnostic flag (allowed: -XX:+UnlockDiagnosticVMOptions, -XX:+SafepointTimeout, \
                -XX:SafepointTimeoutDelay=<ms>, -XX:HandshakeTimeout=<ms>, -Xlog:safepoint*=info,handshake*=info[:file=<path inside the copy>[:decorators]])"));
        }
        out.push(a.to_string());
    }
    Ok(out)
}

fn xlog_ok(a: &str) -> bool {
    let Some(rest) = a.strip_prefix("-Xlog:") else { return false };
    let mut parts = rest.splitn(3, ':');
    let sel = parts.next().unwrap_or("");
    let sel_ok = !sel.is_empty() && sel.split(',').all(|s| {
        let (tags, level) = s.split_once('=').unwrap_or((s, "info"));
        ["info", "debug", "trace"].contains(&level)
            && tags.trim_end_matches('*').split('+').all(|t| t == "safepoint" || t == "handshake")
    });
    let file_ok = match parts.next() {
        None => true,
        Some(o) => o.strip_prefix("file=").is_some_and(|p| {
            !p.is_empty() && p.len() <= 120 && !p.split(['/', '\\']).any(|c| c == "..")
                && p.chars().all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c))
        }),
    };
    let deco_ok = parts.next().is_none_or(|d| d.split(',').all(|x| ["uptime", "time", "tid", "level", "tags", "uptimemillis", "utctime"].contains(&x)));
    sel_ok && file_ok && deco_ok
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

/// Is a game running in this instance? Any java whose command line points into it, as benchlock.py swapwait checks
/// ("/<folder name>" then /, quote, space or the end), so a game Prism started (natives, gameDir) counts too.
fn game_running_in(dir: &Path) -> bool {
    let want = canon(dir).map(|c| c.to_string_lossy().trim_start_matches(r"\\?\").replace('\\', "/").to_ascii_lowercase());
    let Some(want) = want else { return false };
    let key = format!("/{}", want.trim_end_matches('/').rsplit('/').next().unwrap_or_default());
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::All, true, sysinfo::ProcessRefreshKind::new().with_cmd(sysinfo::UpdateKind::Always));
    sys.processes().values().any(|p| {
        if !p.name().to_string_lossy().to_ascii_lowercase().starts_with("java") { return false }
        let line = p.cmd().iter().map(|a| a.to_string_lossy().replace('\\', "/").to_ascii_lowercase()).collect::<Vec<_>>().join(" ");
        line.match_indices(&key).any(|(i, _)| line[i + key.len()..].chars().next().is_none_or(|c| matches!(c, '/' | '"' | ' ')))
    })
}

/// Jars are swapped only while that instance's game is closed (user, KB #36): wait up to 50 s for it to close.
fn swap_wait(dir: &Path) -> Result<(), String> {
    let t0 = Instant::now();
    while game_running_in(dir) {
        if t0.elapsed() > Duration::from_secs(50) {
            let name = dir.file_name().unwrap_or_default().to_string_lossy();
            return Err(format!("the game in {name} is running; nothing was changed. Jars are only swapped while it is closed: try again after it closes (play_wait for the play instance)"));
        }
        std::thread::sleep(Duration::from_secs(5));
    }
    Ok(())
}

// ---------------------------------------------------------------- commands

/// List and object arguments that came as JSON text: an assistant whose tool list predates an option (its MCP helper
/// started before an update) sends a new list option as a string, e.g. "gc": "[\"-XX:+UseG1GC\"]".
fn unstring(args: &Value) -> Value {
    const STRUCTURED: &[&str] = &["gc", "diag", "options", "jvm", "add_mods", "disable_mods", "config", "at", "camera", "test_world"];
    let mut a = args.clone();
    if let Some(map) = a.as_object_mut() {
        for key in STRUCTURED {
            let parsed = map.get(*key).and_then(Value::as_str).map(str::trim)
                .filter(|t| t.starts_with('[') || t.starts_with('{')).and_then(|t| serde_json::from_str::<Value>(t).ok());
            if let Some(v) = parsed { map.insert((*key).to_string(), v); }
        }
    }
    a
}

fn dispatch(cmd: &str, args: &Value, jobs: &Shared, app: &tauri::AppHandle) -> Result<Value, String> {
    if !matches!(cmd, "ping" | "help" | "app-commands" | "app" | "mcp-tools" | "guide") {
        tb_ready()?;
    }
    let args = &unstring(args);
    match cmd {
        // The MCP helper's tool list comes from here, so a running helper picks up an updated Obelisk's tools.
        "mcp-tools" => Ok(json!({ "version": env!("CARGO_PKG_VERSION"), "tools": super::mcp::tools() })),
        "guide" => Ok(json!({ "guide": include_str!("ai-guide.md") })),
        "ping" => Ok(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") })),
        "help" => Ok(json!({ "commands": ["ping", "copies", "launch", "gate", "status", "wait", "kill", "mods", "mod-enable",
            "mod-disable", "mod-add", "mod-remove", "mod-install", "updates", "mod-update", "duplicates", "reports", "diagnose", "log",
            "screenshots", "lightning", "timeline", "copy-pack", "app-commands", "app", "release-submit", "release-status", "release-promote",
            "release-rollback", "release-retry", "release-optout", "errors", "game-query", "game-cmd", "game-scenario", "game-test", "game-result", "game-stop", "game-quit", "bisect"],
            "note": "Every launch waits for the shared bench lock; use wait instead of polling status." })),
        "copies" => copies(),
        // Everything the app itself can do, run live through its window (bridge.rs).
        "app-commands" => Ok(super::bridge::commands()),
        "app" => super::bridge::call(app, args),
        "launch" | "gate" => {
            let r = if cmd == "launch" { launch(args, jobs) } else { gate(args, jobs) };
            if let Some(id) = r.as_ref().ok().and_then(|v| v["job"].as_u64()) { journal::request(id as u32, cmd, args) }
            r
        }
        "errors" => Ok(super::errlog::recent(num(args, "limit", 30, 1, 200)? as usize, args.get("since").and_then(|v| v.as_u64()).unwrap_or(0))),
        "release-submit" | "release-status" | "release-promote" | "release-rollback" | "release-retry" | "release-optout" => super::releases::command(cmd, args),
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
        "play-status" => Ok(play_status()),
        "play-wait" => play_wait(args),
        "kill" => kill(args, jobs),
        "game-test" => {
            let r = game_test(args, jobs);
            // Kept for a restart; submitted again it runs in the background (its caller is gone), read with game_result.
            if let Some(id) = r.as_ref().ok().and_then(|v| v["job"].as_u64()) {
                let mut again = args.clone();
                again["background"] = json!(true);
                journal::request(id as u32, cmd, &again);
            }
            r
        }
        "bisect" => super::bisect::command(args, jobs),
        "game-query" | "game-cmd" | "game-scenario" | "game-quit" => {
            let (id, dir, started) = game_of(args, jobs)?;
            super::game::command(cmd, args, id, &dir, started)
        }
        "game-result" => game_result(args, jobs),
        "game-stop" => {
            let id = args.get("job").and_then(|j| j.as_u64()).ok_or("missing \"job\"")? as u32;
            super::game::progress(id, |p| p.stop = true);
            Ok(json!({ "job": id, "stopping": true, "note": "Automation stops before its next step or second; the game keeps running for game_cmd/game_query. End it with game_quit (measured) or kill." }))
        }
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
            swap_wait(&dir)?;
            let (loader, mc) = pack_of(args)?;
            note_loader_version(&dir);
            let mr = prism::modrinth::Client::new()?;
            let cf = super::cf_client();
            let infos = prism::updates::check(&dir, &super::datadir::root().join("hashes"), &mr, cf.as_ref(), &loader, &mc)?;
            let info = infos.iter().find(|i| i.file_name == file).ok_or_else(|| format!("{file} isn't in that copy"))?;
            let up = info.update.as_ref().ok_or_else(|| format!("{file} is up to date (or not on Modrinth/CurseForge)"))?;
            if up.blocked {
                return Err(format!("CurseForge only allows {} to be downloaded on its website; ask the user", up.file_name));
            }
            // A CurseForge file that's blocked there comes from Modrinth when it's on both.
            let on_mr = if info.provider == "modrinth" { Some((info.project_id.clone(), up.id.clone())) } else { up.modrinth.clone() };
            if let Some((project, version)) = on_mr {
                let project_type = match info.kind.as_str() { "resourcepacks" => "resourcepack", "shaderpacks" => "shader", _ => "mod" };
                let req = prism::modrinth::InstallRequest { project_id: project.clone(), project_type: project_type.into(),
                    version_id: Some(version), loader_override: None, force: false };
                let mut existing = mr.installed(&dir, &super::datadir::root().join("hashes"))?;
                existing.retain(|i| i.file_name != file && i.project_id != project);
                existing.push(prism::modrinth::Installed { project_id: project, version_id: String::new(), kind: info.kind.clone(), file_name: file.into() });
                mr.install(&dir, &req, &loader, &mc, &existing)?;
            } else {
                let cf = cf.ok_or("no CurseForge API key in Obelisk's Settings")?;
                let (m, f) = (info.project_id.parse().map_err(|_| "bad project id")?, up.id.parse().map_err(|_| "bad file id")?);
                if cf.replace(&dir, &info.kind, file, m, f, info.current.parse().unwrap_or(0), &loader, &mc)?.is_some() {
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
            swap_wait(&dir)?;
            let (loader, mc) = pack_of(args)?;
            note_loader_version(&dir);
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
            swap_wait(&dir)?;
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
            swap_wait(&dir)?;
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
            swap_wait(&dir)?;
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
/// (every test copy is a copy of the target pack unless said otherwise).
/// A copy has no Prism instance around it, so the pack check wouldn't know its NeoForge version: note the play
/// pack's (the version copies run) where the check reads it.
fn note_loader_version(dir: &Path) {
    // Only a copy's own note is refreshed (the pack's NeoForge moves on); a real instance knows its own version.
    let noted = dir.join(".obelisk").join("loader-version");
    if !noted.is_file() && !prism::loader_version_of(dir).is_empty() { return }
    if let Some(play) = prism::list_instances(&prism::data_dir()).into_iter().find(|i| i.id == TARGET_PACK) {
        let _ = std::fs::create_dir_all(dir.join(".obelisk"));
        let _ = std::fs::write(dir.join(".obelisk").join("loader-version"), play.loader_version);
    }
}

fn pack_of(args: &Value) -> Result<(String, String), String> {
    let play = prism::list_instances(&prism::data_dir()).into_iter().find(|i| i.id == TARGET_PACK);
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

/// The user's own games: Java started by Prism for a pack in Prism's instances folder (test copies under the
/// bench run from their own folders, so they never count). Read-only: nothing here touches the game.
fn play_games() -> Vec<Value> {
    let data = prism::data_dir();
    let packs: std::collections::HashMap<String, String> = prism::list_instances(&data).into_iter().map(|i| (i.id, i.name)).collect();
    let dir = prism::instances_dir(&data).to_string_lossy().replace('\\', "/").to_lowercase();
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::All, true, sysinfo::ProcessRefreshKind::new().with_cmd(sysinfo::UpdateKind::Always));
    let mut out = vec![];
    for (pid, p) in sys.processes() {
        if !p.name().to_string_lossy().to_ascii_lowercase().starts_with("java") { continue }
        let Some(arg) = p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).find(|a| a.starts_with("-Djava.library.path=")) else { continue };
        if !arg.replace('\\', "/").to_lowercase().contains(&dir) { continue }
        let Some(id) = prism::natives_instance(&arg) else { continue };
        let Some(name) = packs.get(&id) else { continue };
        out.push(json!({ "pack": id, "name": name, "pid": pid.as_u32(), "since": p.start_time(),
            "minutes": now().saturating_sub(p.start_time()) / 60 }));
    }
    out
}

fn play_status() -> Value {
    let games = play_games();
    json!({ "running": !games.is_empty(), "games": games })
}

/// Block until no play game runs (or the timeout passes), then say which. Checks every 2 s inside Obelisk.
fn play_wait(args: &Value) -> Result<Value, String> {
    let secs = num(args, "timeout", 50, 1, 600)?;
    let deadline = Instant::now() + Duration::from_secs(secs);
    let started = Instant::now();
    loop {
        let games = play_games();
        if games.is_empty() {
            return Ok(json!({ "running": false, "closed": true, "waitedSecs": started.elapsed().as_secs() }));
        }
        if Instant::now() >= deadline {
            return Ok(json!({ "running": true, "closed": false, "games": games, "waitedSecs": started.elapsed().as_secs() }));
        }
        std::thread::sleep(Duration::from_secs(2));
    }
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
        let j = l.list.get(&id).ok_or_else(|| journal::gone(id))?;
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

/// One call for a whole test, so a chat spends one turn instead of a launch, waits, log reads and a kill: launch
/// (a fresh world unless one is given, Lightning recording when the copy has it), wait until the player is in the
/// world, run the preset's and/or given steps (game.rs), quit the normal way, and return one compact result: load
/// stages, resources per phase (start, load, world build, in motion, quit), the steps' results, the log's errors by
/// mod and Lightning's summary. background=true returns the job at once; game_result reads it, game_stop takes over.
pub(crate) fn game_test(args: &Value, jobs: &Shared) -> Result<Value, String> {
    let mut la = args.clone();
    let o = la.as_object_mut().ok_or("arguments must be an object")?;
    let preset = o.remove("preset").and_then(|p| p.as_str().map(String::from));
    let dyno = preset.as_deref() == Some("dyno");
    if dyno || preset.as_deref() == Some("stress") {
        o.insert("stop_if_playing".into(), json!(true));
        // Measured runs: 120 s in the world at most and the bench to themselves, like perf runs (launch reads this).
        o.insert("measured".into(), json!(true));
        // Block Dyno has no hold (it quits itself): its limit is the load allowance plus 120 s.
        if dyno { o.insert("timeout".into(), json!(LOAD_ALLOWANCE + SMOKE_HOLD)); }
    }
    let scene = preset.as_deref() == Some("scene");
    // scene: a small flat grass world with time, weather and spawning frozen, for scripted situations (summon, punch,
    // shot steps). Midnight with night vision: as bright as day to the camera, and undead don't burn. The chat's own
    // test_world or world wins.
    if scene && ["test_world", "world", "new_world"].iter().all(|k| o.get(*k).is_none_or(Value::is_null)) {
        o.insert("test_world".into(), json!({ "layers": [{ "block": "minecraft:grass_block", "height": 1 }], "border": 64, "game_mode": "creative",
            "setup": ["gamerule doDaylightCycle false", "time set midnight", "gamerule doWeatherCycle false", "weather clear",
                "gamerule doMobSpawning false", "gamerule doPatrolSpawning false", "gamerule doTraderSpawning false", "kill @e[type=!player]"] }));
    }
    let mut steps: Vec<Value> = match preset.clone().filter(|_| !dyno && !scene) {
        Some(p) => {
            let tests: Vec<String> = o.get("tests").and_then(|t| t.as_array()).into_iter().flatten().filter_map(|t| t.as_str().map(String::from)).collect();
            super::game::preset(&p, &tests)?
        }
        None => vec![],
    };
    o.remove("tests");
    if scene {
        // Shots taken before the terrain is drawn show only fog.
        steps.splice(0..0, [json!({ "cmd": "effect give @s minecraft:night_vision infinite 0 true" }), json!({ "wait_chunks": 20 }), json!({ "settle": 15 })]);
    }
    steps.extend(o.remove("steps").and_then(|s| s.as_array().cloned()).unwrap_or_default());
    let purpose = o.get("purpose").and_then(Value::as_str).unwrap_or_default().to_string();
    // stress and dyno are perf runs in length.
    let long = preset.as_deref().is_some_and(|p| p == "stress" || p == "dyno");
    let test_hold = max_hold(&purpose, long || o.get("perf").and_then(Value::as_bool).unwrap_or(false));
    let scenario_timeout = o.remove("scenario_timeout").and_then(|t| t.as_u64()).unwrap_or(test_hold).min(test_hold);
    let mut background = o.remove("background").and_then(|b| b.as_bool()).unwrap_or(false);
    // A measured run queues until the user is away, maybe for hours: don't hold the chat's call that long.
    let away_wait = if long || o.get("perf").and_then(Value::as_bool).unwrap_or(false) { user_not_away() } else { None };
    if away_wait.is_some() { background = true }
    let mut jvm: Vec<Value> = o.get("jvm").and_then(|j| j.as_array().cloned()).unwrap_or_default();
    let has = |jvm: &[Value], k: &str| jvm.iter().any(|a| a.as_str().is_some_and(|a| a.contains(k)));
    let copy = need(args, "copy")?.to_string();
    let dir = readable(&copy)?;
    let mods_named = |pre: &[&str]| -> Vec<String> {
        std::fs::read_dir(dir.join("mods")).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| { let l = n.to_lowercase(); l.ends_with(".jar") && pre.iter().any(|p| l.starts_with(p)) }).collect()
    };
    if dyno {
        // Block Dyno drives the run itself: its own temp world, its plan, then it quits the game (BlockDyno chat).
        // quick (~3 min) by default: the heavier plans may not finish inside the run limit.
        let mode = o.remove("mode").and_then(|m| m.as_str().map(String::from)).unwrap_or_else(|| "quick".into());
        if !mode.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)) { return Err("mode: a Block Dyno mode or plan name".into()) }
        if !steps.is_empty() { return Err("preset dyno runs Block Dyno's own plan; give steps without it".into()) }
        let jar = tb().root.join("blockdyno").join("build").join("libs").join("blockdyno-1.21.1-0.1.0.jar");
        if !jar.is_file() { return Err(format!("Block Dyno's jar isn't at {}", jar.display())) }
        if mods_named(&["blockdyno"]).is_empty() {
            let mut add = o.get("add_mods").and_then(|a| a.as_array().cloned()).unwrap_or_default();
            add.push(json!(jar.to_string_lossy()));
            o.insert("add_mods".into(), Value::Array(add));
        }
        // It refuses runs capped by Dynamic FPS, and a crashed run's journal would make this one resume the old plan.
        let mut off = o.get("disable_mods").and_then(|a| a.as_array().cloned()).unwrap_or_default();
        off.extend(mods_named(&["dynamic-fps", "dynamic_fps", "dynamicfps"]).into_iter().map(Value::from));
        o.insert("disable_mods".into(), Value::Array(off));
        let _ = std::fs::remove_file(dir.join("blockdyno").join("journal.json"));
        jvm.push(json!(format!("-Dblockdyno.autorun={mode}")));
        jvm.push(json!("-Dblockdyno.label=obelisk"));
    } else if ["test_world", "world", "new_world"].iter().all(|k| o.get(*k).is_none_or(Value::is_null)) {
        // A fresh world (Speed Loader makes it and deletes it after) unless the chat gave one.
        o.insert("new_world".into(), json!(format!("{}", now() % 1_000_000_007)));
    }
    // What the camera shows is the test (scene, shots, recordings): no AFK or respawn cinematic camera taking over
    // an idle player, no teleport fly-in after each tp step, no Dynamic FPS slowing the unfocused window, no chat
    // over the picture. Off for this run only.
    let visual = scene || steps.iter().any(|s| ["shot", "punch", "record"].iter().any(|k| s.get(*k).is_some()));
    if visual {
        let mut off = o.get("disable_mods").and_then(|a| a.as_array().cloned()).unwrap_or_default();
        let add: Vec<Value> = mods_named(&VISUAL_OFF).into_iter().filter(|f| !off.iter().any(|o| o.as_str() == Some(f))).map(Value::from).collect();
        off.extend(add);
        o.insert("disable_mods".into(), Value::Array(off));
        let mut opts = o.get("options").filter(|v| v.is_object()).cloned().unwrap_or_else(|| json!({}));
        if opts.get("chatVisibility").is_none() { opts["chatVisibility"] = json!("2") }
        o.insert("options".into(), opts);
        o.insert("render".into(), json!(true)); // pictures need a drawn frame: parked off-screen, not minimized
    }
    if let Some(tw) = o.get_mut("test_world").filter(|v| v.is_object()) {
        if tw.get("hold").is_none() { tw["hold"] = json!(test_hold) }
    } else if !dyno && !has(&jvm, ".hold=") {
        jvm.push(json!(format!("-Dspeedloader.hold={test_hold}"))); // stays in the world until the steps are done (quit early after)
    }
    let lightning = std::fs::read_dir(dir.join("mods")).into_iter().flatten().flatten()
        .any(|e| { let n = e.file_name().to_string_lossy().to_lowercase(); n.starts_with("lightning") && n.ends_with(".jar") });
    if lightning && !has(&jvm, "lightning.") {
        jvm.push(json!("-Dlightning.capture=gpu,render,ticks,cpu,alloc,gc,threads"));
        jvm.push(json!("-Dlightning.windows=30"));
        jvm.push(json!(format!("-Dlightning.autoStart=1,{RUN_WALL}")));
        o.insert("auto_profile".into(), json!(true));
    }
    o.insert("jvm".into(), Value::Array(jvm));
    let id = launch(&la, jobs)?["job"].as_u64().ok_or("launch gave no job")? as u32;
    super::game::progress(id, |p| { p.phase = "launching".into(); p.steps = steps.len(); });
    let jobs2 = jobs.clone();
    let run = move || {
        let result = run_test(id, steps, scenario_timeout, dyno, &jobs2, &dir).unwrap_or_else(|e| json!({ "job": id, "error": e }));
        super::game::progress(id, |p| { p.phase = "done".into(); p.result = Some(result.clone()); });
        result
    };
    if background {
        std::thread::spawn(run);
        return Ok(json!({ "job": id, "background": true, "queued_until_away": away_wait,
            "note": "game_result {job} for progress or the result; game_stop {job} to take over; game_query/game_cmd work any time." }));
    }
    Ok(run())
}

fn run_test(id: u32, steps: Vec<Value>, scenario_timeout: u64, dyno: bool, jobs: &Shared, dir: &Path) -> Result<Value, String> {
    let t_of = |jobs: &Shared| -> f64 {
        let (m, _) = &**jobs;
        m.lock().unwrap().list.get(&id).and_then(|j| j.now["t"].as_f64()).unwrap_or(0.0)
    };
    // A measured run may sit in the queue for hours (until the user is away).
    let waited = loop {
        let w = wait(&json!({ "job": id, "until": "inWorld", "timeout": 3600 }), jobs)?;
        if w["state"] != "queued" { break w }
    };
    let mut scenario = Value::Null;
    let (mut t_steps, mut t_steps_end, mut t_quit) = (None, None, None);
    let in_world = !dyno && waited["state"] == "running" && waited["events"].as_array().is_some_and(|e| e.iter().any(|e| e["event"] == "inWorld"));
    if in_world {
        super::game::progress(id, |p| p.phase = "in world".into());
        let (_, gdir, started) = game_of(&json!({ "job": id }), jobs)?;
        let t0 = Instant::now();
        let first = loop {
            match super::game::command("game-query", &json!({}), id, &gdir, started) {
                Ok(q) => break Ok(q),
                Err(e) if t0.elapsed() > Duration::from_secs(30) => break Err(e),
                Err(_) => std::thread::sleep(Duration::from_secs(1)),
            }
        };
        t_steps = Some(t_of(jobs));
        scenario = match first {
            Ok(_) if !steps.is_empty() => super::game::scenario(&json!({ "steps": steps, "timeout": scenario_timeout }), id, &gdir, started)
                .unwrap_or_else(|e| json!({ "ok": false, "error": e })),
            Ok(q) => json!({ "ok": true, "query": q }),
            Err(e) => json!({ "ok": false, "error": e }),
        };
        t_steps_end = Some(t_of(jobs));
        // Taken over: the chat drives the game from here and ends it itself.
        if super::game::progress_of(id).is_some_and(|p| p.stop) {
            return Ok(json!({ "job": id, "takenOver": true, "scenario": scenario }));
        }
        super::game::progress(id, |p| p.phase = "quitting".into());
        t_quit = Some(t_of(jobs));
        if super::game::command("game-quit", &json!({}), id, &gdir, started).is_err() {
            let _ = kill(&json!({ "job": id }), jobs);
        }
    }
    if dyno {
        super::game::progress(id, |p| p.phase = "block dyno".into());
        // Block Dyno quits the game itself when its plan is done.
        t_steps = waited["events"].as_array().into_iter().flatten().find(|e| e["event"] == "inWorld").and_then(|e| e["t"].as_f64());
        loop {
            let d = wait(&json!({ "job": id, "timeout": 3600 }), jobs)?;
            if d["state"] != "running" || super::game::progress_of(id).is_some_and(|p| p.stop) { break }
        }
    }
    let mut done = wait(&json!({ "job": id, "timeout": 180 }), jobs)?;
    if done["state"] == "running" {
        let _ = kill(&json!({ "job": id }), jobs);
        done = wait(&json!({ "job": id, "timeout": 30 }), jobs)?;
    }
    let ev = |name: &str| done["events"].as_array().into_iter().flatten().find(|e| e["event"] == name && e["src"] != "optimal").and_then(|e| e["t"].as_f64());
    let stages: serde_json::Map<String, Value> = done["events"].as_array().into_iter().flatten()
        .filter(|e| e["src"] == "log" || e["src"] == "obelisk")
        .filter_map(|e| Some((e["event"].as_str()?.to_string(), e["t"].clone()))).collect();
    // Resources per phase, from Obelisk's once-a-second samples of the game process.
    let samples: Vec<Value> = std::fs::read_to_string(tb().root.join("obelisk-runs").join(done["copy"].as_str().unwrap_or("")).join(format!("{id}.samples.jsonl")))
        .unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let end = samples.last().and_then(|s| s["t"].as_f64()).unwrap_or(0.0);
    let loaded = ev("resourcesLoaded").or(ev("titleScreen"));
    let phases = [
        ("start", Some(0.0), ev("modsLoaded")),
        ("load", ev("modsLoaded"), loaded),
        ("worldBuild", ev("joinStart"), t_steps.or(ev("inWorld"))),
        ("inMotion", t_steps, t_steps_end.or(ev("leaveStart"))),
        ("quit", t_quit, Some(end)),
    ];
    let mut profile = serde_json::Map::new();
    for (name, a, b) in phases {
        let (Some(a), Some(b)) = (a, b) else { continue };
        let w: Vec<&Value> = samples.iter().filter(|s| s["t"].as_f64().is_some_and(|t| t >= a && t <= b)).collect();
        if w.is_empty() { continue }
        let col = |k: &str| -> Vec<f64> { w.iter().filter_map(|s| s[k].as_f64()).collect() };
        let avg = |v: &[f64]| if v.is_empty() { Value::Null } else { json!((v.iter().sum::<f64>() / v.len() as f64).round()) };
        let max = |v: &[f64]| if v.is_empty() { Value::Null } else { json!(v.iter().cloned().fold(f64::MIN, f64::max).round()) };
        let (cpu, mem, heap, gpu, vram, disk) = (col("cpu"), col("memMB"), col("heapUsedMB"), col("gpu3d"), col("vramMB"), col("diskMBps"));
        profile.insert(name.into(), json!({ "seconds": (b - a).round(), "cpuPctAvg": avg(&cpu), "cpuPctMax": max(&cpu),
            "memMaxMB": max(&mem), "heapMaxMB": max(&heap), "gpuPctAvg": avg(&gpu), "vramMaxMB": max(&vram), "diskMBpsAvg": avg(&disk) }));
    }
    // The log in short: verdict, errors grouped by mod, suspects.
    let log = prism::crash::diagnose(dir, "logs/latest.log").map(|d| json!({
        "headline": d.headline,
        "errorsByMod": d.error_groups.iter().take(8).map(|g| json!({ "who": g.who, "count": g.count, "sample": g.sample.chars().take(160).collect::<String>() })).collect::<Vec<_>>(),
        "suspects": d.suspects.iter().take(5).map(|s| json!({ "mod": s.mod_id, "why": s.reason })).collect::<Vec<_>>(),
    })).unwrap_or(Value::Null);
    let lightning = prism::lightning::runs(dir, 1).into_iter().next().map(|r| json!(r)).unwrap_or(Value::Null);
    if dyno { scenario = dyno_result(dir, done["started"].as_u64().unwrap_or(0)) }
    Ok(json!({ "job": id, "state": done["state"], "detail": done["detail"], "stages": stages, "profile": profile,
        "scenario": scenario, "log": log, "lightning": lightning }))
}

/// Block Dyno's newest result from this run, per phase in short (its full result.json and summary.md stay on disk).
fn dyno_result(dir: &Path, since: u64) -> Value {
    let newest = std::fs::read_dir(dir.join("blockdyno").join("results")).into_iter().flatten().flatten()
        .filter(|e| e.path().join("result.json").is_file())
        .filter(|e| e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).is_some_and(|t| t.as_secs() >= since))
        .max_by_key(|e| e.file_name());
    let Some(run) = newest else { return json!({ "ok": false, "error": "Block Dyno saved no result in this run (see log)" }) };
    let r: Value = std::fs::read(run.path().join("result.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    const KEEP: &[&str] = &["name", "seconds", "fps_avg", "fps_1pct_low", "frame_p99_ms", "frame_max_ms", "spikes_over_50ms", "bound",
        "server_tick_mean_ms", "server_tick_max_ms", "heap_peak_mb", "gc_pause_ms", "alloc_stalls", "chunk_loads_per_s", "max_entities", "max_loaded_chunks"];
    let runs: Vec<Value> = r["runs"].as_array().into_iter().flatten().map(|run| json!({
        "variant": run["variant"], "valid": run["valid"], "invalid": run["invalid_reasons"],
        "phases": run["phases"].as_array().into_iter().flatten().map(|p| {
            Value::Object(KEEP.iter().filter_map(|k| p.get(*k).map(|v| (k.to_string(), v.clone()))).collect())
        }).collect::<Vec<_>>(),
    })).collect();
    json!({ "ok": true, "plan": r["plan"]["name"].clone(), "runs": runs, "folder": run.path().to_string_lossy() })
}

/// A background game_test's progress (phase, step, results so far) or its final result.
fn game_result(args: &Value, jobs: &Shared) -> Result<Value, String> {
    let id = args.get("job").and_then(|j| j.as_u64()).ok_or("missing \"job\"")? as u32;
    let p = super::game::progress_of(id).ok_or("no game_test for that job")?;
    if let Some(r) = p.result { return Ok(r) }
    let (m, _) = &**jobs;
    let state = m.lock().unwrap().list.get(&id).map(|j| j.state.clone()).unwrap_or_default();
    Ok(json!({ "job": id, "state": state, "phase": p.phase, "step": p.step, "steps": p.steps, "results": p.results }))
}

/// A running launch or gate job's game folder and start time, for the harness probe.
fn game_of(args: &Value, jobs: &Shared) -> Result<(u32, PathBuf, u64), String> {
    let id = args.get("job").and_then(|j| j.as_u64()).ok_or("missing \"job\"")? as u32;
    let (m, _) = &**jobs;
    let l = m.lock().unwrap();
    let j = l.list.get(&id).ok_or_else(|| journal::gone(id))?;
    if j.state != "running" { return Err(format!("job {id} is {}, not running", j.state)) }
    let dir = match j.kind.as_str() {
        "launch" => Path::new(&j.output).parent().and_then(|p| p.parent()).map(Path::to_path_buf).ok_or("no game folder")?,
        // gate.py takes the first free copy of its pool (gate, gate-2, ...) and names it: "...: gate copy <dir>".
        "gate" => std::fs::read_to_string(&j.output).ok()
            .and_then(|t| t.lines().find_map(|l| l.split_once(": gate copy ").map(|(_, d)| PathBuf::from(d.trim()))))
            .unwrap_or_else(|| tb().root.join("gate")),
        k => return Err(format!("job {id} is a {k}, not a game")),
    };
    Ok((id, dir, j.started))
}

fn kill(args: &Value, jobs: &Shared) -> Result<Value, String> {
    let id = args.get("job").and_then(|j| j.as_u64()).ok_or("missing \"job\"")? as u32;
    let (m, _) = &**jobs;
    let j = m.lock().unwrap().list.get(&id).cloned().ok_or_else(|| journal::gone(id))?;
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
/// Test length limits (user, 2026-11-03 and 2026-11-09 "a tool that limits it"; kb/rules/testing.md): every run holds
/// the world 120 s or less. A non-perf run is killed IN_WORLD_CAP s after it reaches the world (whatever its hold or
/// steps) and never lives past LOAD_ALLOWANCE + IN_WORLD_CAP; a perf (measured) run never past PERF_WALL. Longer work
/// is several short runs. Nothing lives past RUN_WALL either ("overtime").
const SMOKE_HOLD: u64 = 120;
const IN_WORLD_CAP: u64 = 120;
/// Time a run gets after its in-world cap to quit by itself (save, close) before it is killed.
const QUIT_GRACE: u64 = 15;
const PERF_WALL: u64 = 600;
const MAX_TIMEOUT: u64 = 600;
const LOAD_ALLOWANCE: u64 = 240;
const RUN_WALL: u64 = 720;
const OVERTIME: &str = "overtime";

/// The longest hold any run may have: 120 s (the 600 s "interaction" exception is gone, user 2026-11-09).
fn max_hold(_purpose: &str, _perf: bool) -> u64 {
    SMOKE_HOLD
}

/// The wall clock a run gets (seconds after the game starts): perf runs PERF_WALL, others the load allowance plus the
/// in-world cap; a smaller timeout asked for wins.
fn run_cap(measured: bool, asked: u64) -> u64 {
    asked.min(if measured { PERF_WALL } else { LOAD_ALLOWANCE + IN_WORLD_CAP + QUIT_GRACE })
}
/// A run cut short because the user started playing: its numbers don't count.
const INVALID: &str = "invalid";

/// What the user is playing (benchlock.py playing: their launcher's Minecraft outside the bench, or any fullscreen
/// app), or None.
fn user_playing() -> Option<String> {
    let o = bench(&["playing"]).output().ok()?;
    if o.status.code() != Some(0) { return None }
    Some(String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

/// A "fullscreen <app> pid N" entry for a browser or video player: what the user watches, not a game they play.
fn watching_media(entry: &str) -> bool {
    const MEDIA: &[&str] = &["opera", "chrome", "msedge", "firefox", "brave", "vivaldi", "vlc", "mpv", "mpc-hc64", "potplayermini64", "netflix"];
    entry.strip_prefix("fullscreen ").and_then(|r| r.split_whitespace().next()).is_some_and(|app| MEDIA.contains(&app.to_ascii_lowercase().as_str()))
}

/// Why the user isn't away (benchlock.py away: input in the last 10 min, or a game), or None when they are.
fn user_not_away() -> Option<String> {
    let o = bench(&["away"]).output().ok()?;
    if o.status.code() == Some(0) { return None }
    Some(String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty()).or(Some("not away".into()))
}

/// Remove this Obelisk's waiting tickets for `owner` from the bench queue.
fn drop_tickets(owner: &str) {
    let me = std::process::id() as u64;
    for e in std::fs::read_dir(bench_root().join("BENCH.queue")).into_iter().flatten().flatten() {
        let t = std::fs::read(e.path()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        if t.is_some_and(|t| t["owner"].as_str() == Some(owner) && t["pid"].as_u64() == Some(me)) {
            let _ = trash::delete(e.path());
        }
    }
}

/// A running game's heap through jcmd: "histogram" (live objects per class: count and shallow bytes; forces a
/// full GC, takes a few seconds, fine to repeat over time) or "dump" (an .hprof of live objects for MAT or
/// VisualVM; pauses the game for the whole write and is about as big as the used heap). Files go to <copy>/jfr/.
fn heap(args: &Value, jobs: &Shared) -> Result<Value, String> {
    let id = args.get("job").and_then(|j| j.as_u64()).ok_or("missing \"job\"")? as u32;
    let kind = args.get("kind").and_then(|k| k.as_str()).unwrap_or("histogram");
    let j = jobs.0.lock().unwrap().list.get(&id).cloned().ok_or_else(|| journal::gone(id))?;
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
        "heapinfo" => {
            let file = out.join(format!("heapinfo-{}.txt", now()));
            std::fs::write(&file, run(&["GC.heap_info"])?).map_err(|e| e.to_string())?;
            Ok(json!({ "file": file }))
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
    let id = match journal::REUSE.with(|r| r.take()) {
        Some(id) => { l.next = l.next.max(id); id }
        None => { l.next += 1; l.next }
    };
    journal::created(id);
    l.list.insert(id, Job { id, kind: kind.into(), copy: copy.into(), owner: owner.into(), state: "queued".into(), pid: None,
        started: now(), ended: None, detail: String::new(), output, helper_pid: None, java: None, events: vec![], now: Value::Null, hooks: vec![], shots: vec![] });
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
    // Render distance 6 (user, 2026-10-31): light enough for every PC here; a run that needs more asks for it
    // ("options").
    set_option(&options, "renderDistance", TEST_RENDER_DISTANCE);
    // Windowed: a borderless or fullscreen game copied from the play instance covers the desktop and
    // glitches Wallpaper Engine while it's parked off-screen.
    let _ = prism::window::set(dir, prism::window::Mode::Windowed);
    // Configured Defaults puts its own options.txt back over the game's: a fullscreen:true there put a parked test
    // window fullscreen on the user's monitor (Overseer, 2026-11-07).
    let defaults = dir.join("configureddefaults").join("options.txt");
    if defaults.is_file() { set_option(&defaults, "fullscreen", "false"); }
    set_option(&options, "fullscreen", "false");
    afk_cinema_off(dir);
    essential_no_update(dir);
}

/// Essential's loader asks in a window of its own whether to update Essential ("with-prompt"), and a quiet bench
/// launch waits on that window forever. Copies never update it: autoUpdate=false in its own config file
/// (essential/essential-loader.properties), every other line kept. Its loader also rewrites its stage files
/// (essential/loader/stage1/.../stage2.*.properties) to autoUpdate=with-prompt, so every Essential properties file
/// that names autoUpdate gets false too (RT core, 2026-11-06).
fn essential_no_update(dir: &Path) {
    fn walk(d: &Path) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() { walk(&p); continue }
            if p.extension().is_none_or(|x| !x.eq_ignore_ascii_case("properties")) { continue }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            let new: Vec<&str> = text.lines().map(|l| if l.split('=').next().map(str::trim) == Some("autoUpdate") { "autoUpdate=false" } else { l }).collect();
            let new = new.join("\n") + "\n";
            if new.trim_end() != text.trim_end() { let _ = std::fs::write(&p, new); }
        }
    }
    let installed = dir.join("essential").is_dir() || std::fs::read_dir(dir.join("mods")).into_iter().flatten().flatten()
        .any(|e| e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("essential"));
    if !installed { return }
    let file = dir.join("essential").join("essential-loader.properties");
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().filter(|l| l.split('=').next().map(str::trim) != Some("autoUpdate")).map(String::from).collect();
    lines.push("autoUpdate=false".into());
    let _ = std::fs::create_dir_all(dir.join("essential"));
    let _ = std::fs::write(&file, lines.join("\n") + "\n");
    walk(&dir.join("essential"));
    // Online too: a copy runs signed out (offline name), so Essential's network sign-in fails and puts "Account
    // authentication failed" over the title screen, which held a world launch there until the hang check killed
    // it (Reinforcements, 2026-11-06). Essential's own "enable Essential" switch off stops its connection loop
    // before it signs in; the mod itself still loads.
    let cfg = dir.join("essential").join("config.toml");
    if let Ok(text) = std::fs::read_to_string(&cfg) {
        let new: Vec<String> = text.lines().map(|l| match l.split('=').next().map(str::trim) {
            Some("enable_essential") => format!("{}enable_essential = false", &l[..l.len() - l.trim_start().len()]),
            // 1 copies every screenshot to the Windows clipboard, so test shots overwrote what the user had copied
            // (Overseer, 2026-11-07). The user's own instances keep their choice; copies never touch the clipboard.
            Some("post_screenshot_action") => format!("{}post_screenshot_action = 0", &l[..l.len() - l.trim_start().len()]),
            _ => l.to_string(),
        }).collect();
        let new = new.join("\n") + "\n";
        if new.trim_end() != text.trim_end() { let _ = std::fs::write(&cfg, new); }
    }
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

/// The render distance every test copy runs at unless a run asks for another ("options").
const TEST_RENDER_DISTANCE: &str = "6";

/// Properties that make a launch an FPS measurement (Lightning captures, pack and load benchmarks).
const FPS_PROPS: &[&str] = &["-Dlightning.capture", "-Dpackbench.", "-Dspeedloader.autobench"];
/// Packbench settings that only set up a picture (a scene, a world, screenshots), not a measurement: they need a drawn
/// frame (parked off-screen, full size) but run like any quick test (Overseer, 2026-11-07).
const CAPTURE_PROPS: &[&str] = &["-Dpackbench.shots", "-Dpackbench.scene", "-Dpackbench.world", "-Dpackbench.tag", "-Dpackbench.idleTicks"];

/// Whether this JVM argument makes the run an FPS measurement.
fn fps_prop(a: &str) -> bool {
    FPS_PROPS.iter().any(|p| a.starts_with(p)) && !CAPTURE_PROPS.iter().any(|p| a.starts_with(p))
}
/// The frame cap an FPS run uses: high enough that no laptop or desktop here reaches it, so the numbers are the game's.
const FPS_RUN_MAX: &str = "260";
/// Every other test: a light window and frame cap so tests don't eat the PC (user, KB #139).
const TEST_FPS: &str = "60";
const TEST_SIZE: (u32, u32) = (1280, 720);
/// Where a full-pack check run's heap tries to stay (it may still grow to its -Xmx).
const FULL_PACK_SOFT_MB: u64 = 4096;

/// A lean copy's heap in MB (leancopy.py writes heap_gb to lean.json; 2 GB if it doesn't say). None for other copies.
fn lean_heap_mb(dir: &Path) -> Option<u64> {
    let meta: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("lean.json")).ok()?).unwrap_or(Value::Null);
    Some((meta["heap_gb"].as_f64().filter(|g| *g >= 1.0).unwrap_or(2.0) * 1024.0) as u64)
}

fn get_option(path: &Path, key: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()?.lines().find_map(|l| l.strip_prefix(&format!("{key}:")).map(str::to_string))
}

/// An FPS run measures the game, not a cap: vsync off (Minecraft's default is on, which holds a run at the monitor's
/// 60 Hz) and the frame limit at FPS_RUN_MAX. Returns what the copy had, for `uncap_undo` after the run.
fn uncap(options: &Path) -> Vec<(String, Option<String>)> {
    set_for_run(options, &[("enableVsync".into(), "false".into()), ("maxFps".into(), FPS_RUN_MAX.into())])
}

/// Set options for one run; returns what the copy had, for `uncap_undo` after it.
fn set_for_run(options: &Path, set: &[(String, String)]) -> Vec<(String, Option<String>)> {
    set.iter().map(|(key, value)| {
        let before = get_option(options, key);
        set_option(options, key, value);
        (key.clone(), before)
    }).collect()
}

/// This run's options ("options": {"renderDistance": 12, ...}), set for the run and put back after it.
fn run_options(args: &Value) -> Result<Vec<(String, String)>, String> {
    let Some(map) = args.get("options").filter(|v| !v.is_null()) else { return Ok(vec![]) };
    let map = map.as_object().ok_or("\"options\" must be an object of options.txt keys, e.g. {\"renderDistance\": 12}")?;
    map.iter().map(|(k, v)| {
        let value = match v { Value::String(s) => s.clone(), Value::Bool(_) | Value::Number(_) => v.to_string(), _ => return Err(format!("option {k}: give a text, number or true/false")) };
        if k.is_empty() || k.contains([':', '\n', '\r']) || value.contains(['\n', '\r']) { return Err(format!("option {k:?}: not an options.txt key and value")) }
        Ok((k.clone(), value))
    }).collect()
}

/// Put back what the copy had before a run's settings (the game may have rewritten the file meanwhile; only these keys change).
fn uncap_undo(options: &Path, before: &[(String, Option<String>)]) {
    for (key, value) in before {
        match value {
            Some(v) => set_option(options, key, v),
            None => {
                let Ok(text) = std::fs::read_to_string(options) else { continue };
                let kept: Vec<&str> = text.lines().filter(|l| !l.starts_with(&format!("{key}:"))).collect();
                let _ = std::fs::write(options, kept.join("\n") + "\n");
            }
        }
    }
}

/// The FPS run's line: the vsync and frame cap it really ran with (Lightning's capture says, else options.txt), and a
/// warning when the frames still look capped (median at 16.6 ms, 60 FPS, with more time waiting on the swap than the GPU took).
fn fps_report(dir: &Path, since_ms: u64) -> String {
    let options = dir.join("options.txt");
    let run = prism::lightning::runs(dir, 1).into_iter().find(|r| r.started_ms >= since_ms.saturating_sub(5_000));
    let raw = run.as_ref().and_then(|r| std::fs::read(dir.join("lightning").join("captures").join(format!("lightning-{}.results.json", r.id))).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok()).unwrap_or(Value::Null);
    let vsync = raw["run"]["vsync"].as_bool().map(|v| v.to_string()).or_else(|| get_option(&options, "enableVsync")).unwrap_or("?".into());
    let max = raw["run"]["fpsLimit"].as_u64().map(|v| v.to_string()).or_else(|| get_option(&options, "maxFps")).unwrap_or("?".into());
    let source = if raw["run"].is_object() { "per Lightning" } else { "per options.txt" };
    let mut line = format!("vsync {vsync}, maxFps {max} ({source})");
    let w = &raw["whole"];
    if let (Some(p50), Some(swap), Some(gpu)) = (w["frameP50"].as_f64(), w["swapWaitP50"].as_f64(), w["gpuFrameP50"].as_f64()) {
        if (16.3..=17.0).contains(&p50) && swap > gpu {
            line.push_str(&format!("; WARNING: frames look capped at 60 FPS (frameP50 {p50:.1} ms, swapWait {swap:.1} ms > GPU {gpu:.1} ms): some cap is on (driver vsync, a frame limiter or the monitor), so these FPS numbers aren't the game's"));
        }
    }
    line
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
        // Else the version noted in the copy, else the one the target pack is on now: a copy runs the NeoForge its pack
        // plays on, when the bench has it installed.
        .or_else(|| {
            let noted = std::fs::read_to_string(dir.join(".obelisk").join("loader-version")).ok().map(|v| v.trim().to_string());
            let play = || prism::list_instances(&prism::data_dir()).into_iter().find(|i| i.id == TARGET_PACK).map(|i| i.loader_version);
            [noted, play()].into_iter().flatten()
                .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.'))
                .map(|v| format!("neoforge-{v}"))
                .find(|n| Path::new(&tb().install).join("versions").join(n).join(format!("{n}.json")).is_file())
        })
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
        "-Dtestharness.background=true",
        // Essential's stage1 loader still shows its update prompt when autoUpdate=false is ignored; a download
        // address that refuses at once means there's never an update to offer (Crashaholic, 2026-11-02).
        "-Dessential.download.url=http://127.0.0.1:9",
        // The harness answers game_query/game_cmd/game_scenario on a localhost socket (game.rs).
        "-Dtestharness.probe=true"].iter().map(|s| s.to_string()).collect();
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

/// The cross-PC scenario (same on every PC, so two runs compare mod by mod): a fixed-seed normal world flown
/// through in spectator on a fixed route, the same options and Lightning capture, and no per-PC overrides.
/// Anything the call gives itself wins over the preset (e.g. config for a shader pass).
pub(crate) const CROSSPC: &str = "crosspc-2"; // 2: 120 s in the world (user, 2026-11-03), not comparable with crosspc-1
fn crosspc_args(args: &Value) -> Value {
    let preset = json!({
        "perf": true, "eta": 8, "timeout": 600,
        "test_world": {
            "terrain": "normal", "seed": "obelisk-crosspc", "game_mode": "spectator", "hold": 120,
            "spawn": { "x": 0, "y": 140, "z": 0 },
            "setup": ["gamerule doDaylightCycle false", "time set 6000", "weather clear", "gamerule doWeatherCycle false"],
            "events": [
                { "at": 30, "command": "tp @a 512 140 0 90 20" },
                { "at": 50, "command": "tp @a 512 140 512 0 20" },
                { "at": 70, "command": "tp @a 0 140 512 -90 20" },
                { "at": 90, "command": "tp @a -512 140 -512 135 20" }
            ]
        },
        "options": { "enableVsync": false, "maxFps": 260, "renderDistance": 6, "simulationDistance": 6 },
        // Lightning records from 35 s in the world for 150 s (all four legs of the route), then quits the game.
        "jvm": ["-Dlightning.capture=gpu,render,ticks,cpu,alloc,gc,threads", "-Dlightning.crosspc=true", "-Dlightning.autoStart=15,100,quit"],
    });
    let mut out = preset;
    for (k, v) in args.as_object().into_iter().flatten() {
        if k != "preset" { out[k] = v.clone(); }
    }
    out
}

fn launch(args: &Value, jobs: &Shared) -> Result<Value, String> {
    let crosspc = match args.get("preset").and_then(Value::as_str) {
        None => false,
        Some("crosspc") => true,
        Some(p) => return Err(format!("unknown preset {p:?} (there is: crosspc)")),
    };
    let expanded;
    let args = if crosspc { expanded = crosspc_args(args); &expanded } else { args };
    let name = need(args, "copy")?.to_string();
    let dir = test_copy(&name)?;
    prism::dupes::check_before_launch(&dir)?;
    let owner = label(need(args, "owner")?, "owner")?;
    let purpose = label(need(args, "purpose")?, "purpose")?;
    let eta = num(args, "eta", 5, 1, 30)?;
    let timeout = num(args, "timeout", MAX_TIMEOUT, 60, MAX_TIMEOUT)?;
    // Always in the background (user, 2026-10-24): "park": false is no longer honoured.
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
    let mut build_world: Option<(Value, u64)> = None;
    let args_world = match args.get("test_world").filter(|v| !v.is_null()) {
        Some(spec) => {
            if s(args, "world").is_some() || seed_arg(args).is_some() {
                return Err("give test_world, world or new_world, not more than one".into());
            }
            if game_running_in(&dir) {
                return Err(format!("a game is already running in {name}"));
            }
            // Only planned here: the world is built once the run holds the bench lock, since building one clears
            // the copy's earlier test worlds and a queued run must not pull the world from under a running one.
            let stamp = now();
            let (save, hold) = prism::testworld::plan(spec, stamp)?;
            build_world = Some((spec.clone(), stamp));
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
    let new_world_arg = seed_arg(args);
    let (world, seed) = match args_world.map(|w| w.strip_prefix("new:").ok_or(w)) {
        Some(Ok(seed)) => (None, Some(seed)),
        Some(Err(w)) => (Some(w), new_world_arg.as_deref()),
        None => (None, new_world_arg.as_deref()),
    };
    if let Some(w) = world {
        plain_name(w, "world").map_err(|_| "world must be a plain save folder name".to_string())?;
        // A test world is built later, once the run holds the lock.
        if build_world.is_none() && !dir.join("saves").join(w).is_dir() {
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
        // The recording also streams to its own repository, flushed every second: a game that is killed (no dump
        // on exit) still leaves everything up to its last second there, and stitch_jfr turns that into the file.
        let (out, stamp) = (jfr.to_string_lossy().replace('\\', "/"), now());
        extra.push(format!("-XX:FlightRecorderOptions:repository={out}/{JFR_REPO}{stamp}"));
        // Its own name: the Lightning mod takes over a recording called "lightning" and stops it when its start-up
        // capture ends, which cut this one off at the first join.
        extra.push(format!("-XX:StartFlightRecording=name=obelisk,settings=profile,class-loading=true{roots},flush-interval=1s,filename={out}/obelisk-{stamp}.jfr,dumponexit=true"));
    }
    let mut swap = ModSwap::plan(&dir, args)?;
    // Benchmark run conditions (user, 2026-11-02): no Dynamic FPS (it throttles an unfocused window) and no Absolute
    // AFK Cinema, both set aside for the run only; the game runs at the monitor's native resolution.
    if args.get("perf").and_then(|v| v.as_bool()).unwrap_or(false) {
        for e in std::fs::read_dir(dir.join("mods")).into_iter().flatten().flatten() {
            let f = e.file_name().to_string_lossy().into_owned();
            let low = f.to_ascii_lowercase();
            if low.ends_with(".jar") && PERF_OFF.iter().any(|m| low.starts_with(m)) && !swap.disable.contains(&f) {
                swap.disable.push(f);
            }
        }
    }
    let hooks = plan_hooks(args)?;
    if game_running_in(&dir) {
        return Err(format!("a game is already running in {name}"));
    }
    // Build the command now so a broken copy fails before queueing.
    // Holds: what the chat asked (jvm), a test_world's own, or the default one put in above.
    let perf_run = args.get("perf").and_then(|v| v.as_bool()).unwrap_or(false);
    // game_test records Lightning on every run it can: that alone doesn't make it a measured run.
    let auto_profile = args.get("auto_profile").and_then(|v| v.as_bool()).unwrap_or(false);
    let fps_props = extra.iter().any(|a| fps_prop(a) && !a.starts_with("-Dspeedloader.autobench")
        && !(auto_profile && a.starts_with("-Dlightning.")));
    let measured = perf_run || fps_props || args.get("measured").and_then(|v| v.as_bool()).unwrap_or(false);
    // Frame times and pictures need a drawn frame: those windows park off-screen instead of minimized.
    let render = measured || extra.iter().any(|a| CAPTURE_PROPS.iter().any(|p| a.starts_with(p)))
        || args.get("render").and_then(|v| v.as_bool()).unwrap_or(false);
    let cap = max_hold(&purpose, measured);
    let mut hold = None;
    for a in &extra {
        let Some(v) = a.strip_prefix("-Dspeedloader.hold=").or_else(|| a.strip_prefix("-Doptimal.hold=")) else { continue };
        let h: u64 = v.trim().parse().map_err(|_| format!("{a}: the hold is whole seconds"))?;
        if h > cap {
            return Err(format!("hold {h} s is over the limit of {cap} s for every run (kb/rules/testing.md): split longer work into several short runs"));
        }
        hold = Some(hold.map_or(h, |o: u64| o.max(h)));
    }
    // An autobench run without Optimal would sit on the title screen until the hang check (Overseer, 2026-11-03).
    if extra.iter().any(|a| a.starts_with("-Dspeedloader.autobench")) {
        let autobench = |n: &str| { let n = n.to_lowercase(); (n.starts_with("optimal") || n.starts_with("speedloader")) && n.ends_with(".jar") };
        // One set aside by disable_mods doesn't count: the run would load without it (Reinforcements job 12, 2026-11-06).
        let off: Vec<String> = args.get("disable_mods").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_lowercase).collect();
        let in_mods = std::fs::read_dir(dir.join("mods")).into_iter().flatten().flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .any(|n| autobench(&n) && !off.contains(&n.to_lowercase()));
        let added = args.get("add_mods").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str)
            .any(|p| Path::new(p).file_name().is_some_and(|f| autobench(&f.to_string_lossy())));
        if !in_mods && !added { return Err(format!("no autobench mod (Optimal) in copy {name}")) }
    }
    // The hard wall clock: load allowance plus the hold, never past RUN_WALL; with no hold, the time limit.
    let timeout = match hold {
        Some(h) => {
            let wall = (LOAD_ALLOWANCE + h).min(RUN_WALL);
            if args.get("timeout").is_some_and(|v| !v.is_null()) { wall.min(timeout.max(h + 60)) } else { wall }
        }
        None => timeout,
    };
    let timeout = run_cap(measured, timeout);
    let world_cap = (!measured).then_some(IN_WORLD_CAP);
    let eta = eta.min(timeout.div_ceil(60) + 1);
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
    // Tells the game itself not to take the focus when its window appears (Optimal reads it: GLFW focus-on-show off,
    // and iconified right after creation unless it has to draw). Obelisk then touches each window once at most.
    // Always "unfocused", never "minimized": the taskbar preview has to stay live (user, 11-11).
    cmd.insert(0, "-Dobelisk.background=unfocused".to_string());
    if args.get("perf").and_then(|v| v.as_bool()).unwrap_or(false) {
        if let Some((w, h)) = native_resolution() {
            cmd.extend(["--width".into(), w.to_string(), "--height".into(), h.to_string()]);
        }
    } else {
        // Tests must not eat the PC (user, KB #139): a 1280x720 window, and a lean copy's own small heap (lean.json
        // heap_gb, else 2 GB) unless the chat set -Xmx itself. Full-pack copies keep their 6 GB budget.
        cmd.extend(["--width".into(), TEST_SIZE.0.to_string(), "--height".into(), TEST_SIZE.1.to_string()]);
        if let Some(mx) = lean_heap_mb(&dir).filter(|_| !extra.iter().any(|a| a.starts_with("-Xmx"))) {
            let ms = cmd.iter().find_map(|a| heap_flag(a).filter(|(k, _)| *k == "-Xms").and_then(|(_, v)| v));
            cmd.retain(|a| !heap_flag(a).is_some_and(|(k, _)| k == "-Xmx" || (k == "-Xms" && ms.is_none_or(|m| m > mx))));
            cmd.insert(0, format!("-Xmx{mx}m"));
            if !cmd.iter().any(|a| a.starts_with("-Xms")) { cmd.insert(0, "-Xms256m".into()) }
        } else if cmd.iter().any(|a| a == "-XX:+UseZGC") && !cmd.iter().any(|a| a.starts_with("-XX:SoftMaxHeapSize")) {
            // A full pack in the world uses up to about 4 GB (jobs 581-597): ZGC keeps under that and gives memory
            // back, and still grows to -Xmx rather than run out.
            cmd.insert(0, format!("-XX:SoftMaxHeapSize={FULL_PACK_SOFT_MB}m"));
        } else if cmd.iter().any(|a| a == "-XX:+UseG1GC") && !cmd.iter().any(|a| a.starts_with("-XX:G1PeriodicGCInterval")) {
            // G1 has no soft limit: a collection every 30 s when it's quiet hands unused memory back (Overseer, run 618).
            cmd.insert(0, "-XX:G1PeriodicGCInterval=30000".into());
        }
    }
    if prism::java::at(Path::new(&java)).is_some_and(|j| j.major >= 24) {
        cmd.retain(|a| a != "-XX:+ZGenerational"); // ZGC is generational-only from Java 24
    }
    if args.get("compact_headers").and_then(|v| v.as_bool()).unwrap_or(false) {
        if !java.contains("jdk-25") { return Err("compact_headers needs java 25".into()); }
        cmd.insert(0, "-XX:+UseCompactObjectHeaders".into());
    }
    // A GC A/B run: every GC setting from the profile and the bench defaults out, this run's in.
    let gc = gc_flags(args)?;
    if let Some(flags) = &gc {
        // A heap size given here replaces the profile's and the bench's of the same kind.
        let kinds: Vec<&str> = flags.iter().filter_map(|f| heap_flag(f).map(|(k, _)| k)).collect();
        cmd.retain(|a| (!is_gc_flag(a) || a == "-XX:+UnlockExperimentalVMOptions") && !heap_flag(a).is_some_and(|(k, _)| kinds.contains(&k)));
        for (i, a) in flags.iter().enumerate() { cmd.insert(i, a.clone()) }
    }
    for (i, a) in diag_flags(args)?.into_iter().enumerate() { cmd.insert(i, a) }
    let gc_used = gc.map(|f| f.join(" "));
    let set = run_options(args)?;
    let lock = format!("{owner} - {purpose} (obelisk)");
    let id = new_job(jobs, "launch", &name, &lock, dir.join("logs").join("latest.log").to_string_lossy().into_owned());
    let jobs = jobs.clone();
    // A comparison (perf, a GC A/B, or "ab": before/after) needs the bench to itself: it queues until the user is
    // away and ends "invalid" when they come back (user, KB #36). Any other run whose result is a number (fps,
    // Lightning captures) shares the PC with at most one other game (benchlock --measured, KB #91); the rest share
    // it, one run per copy.
    let perf = args.get("perf").and_then(|v| v.as_bool()).unwrap_or(false);
    let exclusive = measured && (perf || gc_used.is_some() || args.get("ab").and_then(|v| v.as_bool()).unwrap_or(false));
    let measured_shared = measured && !exclusive;
    let stop_if_playing = measured || args.get("stop_if_playing").and_then(|v| v.as_bool()).unwrap_or(false);
    let reply_gc = gc_used.clone();
    // Only measurements uncap frames. Joining a world (autobench) is how every game_test gets in, so it isn't one
    // (Overseer blind test, run 616: check runs were left at 260 fps).
    let fps_run = measured;
    std::thread::spawn(move || run_game(jobs, id, dir, lock, eta, timeout, world_cap, render, perf, exclusive, measured_shared, stop_if_playing, java, cmd, swap, new_world, hooks, gc_used, set, !crosspc, build_world, fps_run));
    let lock_note = if exclusive { format!("comparison run: queued until the user is away (benchlock.py away), as \"{owner} - {purpose} (obelisk)\"") }
        else if measured_shared { format!("measured run: shares the PC with at most one other game, as \"{owner} - {purpose} (obelisk)\"") }
        else { format!("waiting for the bench lock as \"{owner} - {purpose} (obelisk)\"") };
    Ok(json!({ "job": id, "state": "queued", "lock": lock_note, "gc": reply_gc,
        "limit": match world_cap {
            Some(c) => format!("CAP: {c} s in the world (killed {QUIT_GRACE} s later if still there), {timeout} s in all, then \"{OVERTIME}\"; split longer work into several runs"),
            None => format!("CAP (perf): {timeout} s in all after the game starts, then \"{OVERTIME}\""),
        },
        "preset": crosspc.then_some(CROSSPC) }))
}

#[allow(clippy::too_many_arguments)]
fn run_game(jobs: Shared, id: u32, dir: PathBuf, lock: String, eta: u64, timeout: u64, world_cap: Option<u64>, render: bool, perf: bool, exclusive: bool, measured: bool, stop_if_playing: bool, java: String, cmd: Vec<String>,
    mut swap: ModSwap, new_world: bool, mut hooks: Vec<Hook>, gc_used: Option<String>, set: Vec<(String, String)>, pc_overrides: bool,
    build_world: Option<(Value, u64)>, fps_run: bool) {
    let me = std::process::id().to_string();
    let copy = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let eta_s = eta.to_string();
    let mut acquire = vec!["acquire", &lock, "--pid", &me, "--wait", "--eta", &eta_s];
    if !exclusive { acquire.extend(["--shared", "--copy", &copy]); }
    if measured { acquire.push("--measured"); }
    let child = bench(&acquire).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    let Ok(mut child) = child else {
        update(&jobs, id, |j| { j.state = "failed".into(); j.detail = "couldn't run benchlock.py".into(); j.ended = Some(now()); });
        return;
    };
    update(&jobs, id, |j| j.helper_pid = Some(child.id()));
    // While queued, say who holds the lock and until when, so a wait shows why it's slow.
    let got = loop {
        if let Ok(Some(s)) = child.try_wait() { break s.code() == Some(0); }
        let queue = bench(&["queue"]).output().ok().and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok());
        let d = match (if exclusive { user_not_away() } else { user_playing() }, queue) {
            (Some(p), _) if exclusive => Some(format!("waiting until the user is away ({p})")),
            (Some(p), _) => Some(format!("waiting: the user is playing ({p})")),
            (None, Some(q)) => {
                // Only what really blocks this run: the exclusive holder, shared runs in this copy (all of them for a
                // perf run), or a perf run queued first (it only goes first once the user is away).
                let held = &q["held"];
                let shared: Vec<String> = q["shared"].as_array().into_iter().flatten()
                    .filter(|s| exclusive || s["copy"].as_str() == Some(copy.as_str()))
                    .map(|s| format!("{} in {} (until {})", s["owner"].as_str().unwrap_or("?"), s["copy"].as_str().unwrap_or("?"),
                        s["expected_end"].as_str().unwrap_or("?"))).collect();
                let perf: Vec<&str> = q["waiting"].as_array().into_iter().flatten()
                    .filter(|w| w["copy"].is_null() && w["owner"].as_str() != Some(lock.as_str())).filter_map(|w| w["owner"].as_str()).collect();
                Some(if let Some(o) = held["owner"].as_str() {
                    format!("waiting for the bench lock: {o} has it, expected to end {}", held["expected_end"].as_str().unwrap_or("?"))
                } else if !shared.is_empty() {
                    format!("waiting for the bench lock: shared by {}", shared.join(", "))
                } else if !exclusive && !perf.is_empty() {
                    format!("waiting for the bench lock: perf runs go first while the user is away ({})", perf.join(", "))
                } else {
                    // Otherwise it's first come, first served: name the runs queued ahead of this one.
                    let waiting: Vec<&str> = q["waiting"].as_array().into_iter().flatten().filter_map(|w| w["owner"].as_str()).collect();
                    match waiting.iter().position(|o| *o == lock.as_str()) {
                        Some(n) if n > 0 => format!("waiting for the bench lock: {n} run{} queued ahead, first {}", if n == 1 { "" } else { "s" }, waiting[0]),
                        _ => format!("waiting for the bench lock (checking that no game is running in {copy})"),
                    }
                })
            }
            _ => None,
        };
        if let Some(d) = d { update(&jobs, id, |j| if j.detail != KILLED { j.detail = d }); }
        std::thread::sleep(Duration::from_secs(15));
    };
    update(&jobs, id, |j| if j.detail.starts_with("waiting for the bench lock") || j.detail.starts_with("waiting: the user is playing") { j.detail.clear() });
    update(&jobs, id, |j| j.helper_pid = None);
    if !got {
        // The ticket names Obelisk's own pid, which stays alive, so benchlock.py never reaps it: a killed or failed
        // wait would hold its place in the queue for good (and a perf ticket keeps every shared run waiting).
        drop_tickets(&lock);
        finish(&jobs, id, "failed", "didn't get the bench lock".into());
        return;
    }
    let saves_before = if new_world { save_names(&dir) } else { Default::default() };
    let fps_run = perf || fps_run;
    // Only real measurements stop when the user starts playing; the caller already worked that out (`measured` covers
    // FPS props). Re-scanning cmd here also caught game_test's automatic Lightning recording, so every check run with
    // Lightning was killed the moment the user launched their game (user 11-10: chats waiting on runs that never come).
    let stop_if_playing = stop_if_playing || perf;
    let caps = std::sync::Mutex::new(None);
    let run_set = std::sync::Mutex::new(None);
    let result = (|| -> Result<String, String> {
        if let Some((spec, stamp)) = &build_world {
            prism::testworld::build(&dir, "1.21.1", spec, *stamp)?;
        }
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
        // This PC's own lighter settings for the copy (pc-overrides.json), keyed by the copy's name.
        let copy = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let copy = if copy.eq_ignore_ascii_case("minecraft") || copy.eq_ignore_ascii_case(".minecraft") {
            dir.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or(copy)
        } else { copy };
        if pc_overrides { for p in super::apply_pc_overrides(&dir, &[&copy]) { eprintln!("pc override: {p}") } }
        let opts = dir.join("options.txt");
        *caps.lock().expect("caps") = Some(if fps_run { uncap(&opts) } else {
            set_for_run(&opts, &[("enableVsync".into(), "false".into()), ("maxFps".into(), TEST_FPS.into())])
        });
        if !set.is_empty() { *run_set.lock().expect("run options") = Some(set_for_run(&dir.join("options.txt"), &set)); }
        // Run logs live beside the copies, never inside one: a copy's folder must match the pack it came from.
        let runs = tb().root.join("obelisk-runs").join(dir.file_name().unwrap_or_default());
        let _ = std::fs::create_dir_all(&runs);
        let out = std::fs::File::create(runs.join(format!("{id}.stdout"))).map_err(|e| e.to_string())?;
        // The exact Java and arguments, for when a run dies before the game writes any log.
        let used = ["maxFps", "enableVsync"].map(|k| format!("{k}:{}", get_option(&opts, k).unwrap_or("?".into()))).join(" ");
        let _ = std::fs::write(runs.join(format!("{id}.cmd.txt")), format!("{java}\n{}\n# options for this run: {used}\n", cmd.join("\n")));
        let started = SystemTime::now();
        let click_ms = prism::timeline::now_ms();
        let mut game = Command::new(&java);
        game.args(&cmd).current_dir(&dir).stdout(out.try_clone().map_err(|e| e.to_string())?).stderr(out);
        // Below-normal from the first instruction, so the load spike never beats the user's mouse (KB #137).
        { use std::os::windows::process::CommandExt; game.creation_flags(0x0800_0000 /* CREATE_NO_WINDOW */ | 0x4000 /* BELOW_NORMAL */); }
        let mut game = game.spawn().map_err(|e| format!("couldn't start Java: {e}"))?;
        let pid = game.id();
        // The CPU ceiling (KB #138) is ResCap's job, not Obelisk's: job objects and affinity changes on a child
        // process got Obelisk quarantined as a trojan (Defender ML, 11-12). Below-normal priority stays (creation flag).
        TEST_PIDS.lock().expect("test pids").insert(pid);
        let mut retakes = HashMap::new();
        update(&jobs, id, |j| { j.state = "running".into(); j.pid = Some(pid); j.java = Some(java.clone()); });
        let t0 = Instant::now();
        let mut parked = std::collections::HashSet::new();
        let mut prev_fg = foreground_window();
        // Whether the user is in their own game: then a test window never keeps the focus (user, 11-10: keys stuck
        // mid-play when one grabbed it). Checked every 15 s while this run lasts.
        let user_game = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let run_live = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        {
            let (user_game, run_live) = (user_game.clone(), run_live.clone());
            std::thread::spawn(move || while run_live.load(std::sync::atomic::Ordering::Relaxed) {
                let p = user_playing().is_some_and(|p| p.split("; ").any(|e| !watching_media(e)));
                user_game.store(p, std::sync::atomic::Ordering::Relaxed);
                std::thread::sleep(Duration::from_secs(15));
            });
        }
        let mut stopping: Option<Instant> = None;
        let mut window_ms = None;
        let mut watch = super::devrun::Sampler::new(pid, &dir, &runs, id, click_ms, super::devrun::jcmd_for(&java).ok());
        let mut shown = Instant::now();
        let latest = dir.join("logs").join("latest.log");
        let stdout_file = runs.join(format!("{id}.stdout"));
        let mut stall = Stall::default();
        let mut frozen_since: Option<Instant> = None;
        let mut entered: Option<Instant> = None;
        // Numbers taken while the user plays are wrong and the run hurts their game (user, 2026-11-03; KB #31).
        let playing = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        let watching = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(stop_if_playing));
        if stop_if_playing {
            let (playing, watching) = (playing.clone(), watching.clone());
            std::thread::spawn(move || while watching.load(std::sync::atomic::Ordering::Relaxed) {
                // A quick run only steps aside for a game: a fullscreen browser or video player isn't one (Boundless, 2026-11-07).
                let p = if exclusive { user_not_away() } else { user_playing().filter(|p| p.split("; ").any(|e| !watching_media(e))) };
                if let Some(p) = p { *playing.lock().expect("playing") = Some(p); break }
                std::thread::sleep(Duration::from_secs(10));
            });
        }
        let why = loop {
            if let Ok(Some(status)) = game.try_wait() {
                let _ = std::fs::write(runs.join(format!("{id}.exit")), status.code().map(|c| c.to_string()).unwrap_or("?".into()));
                break format!("exited ({})", status.code().map(|c| c.to_string()).unwrap_or("?".into()));
            }
            let guard = t0.elapsed() < FOCUS_GUARD || user_game.load(std::sync::atomic::Ordering::Relaxed);
            park_windows(pid, &mut parked, render, if perf { native_resolution() } else { Some(TEST_SIZE) }, &mut prev_fg, guard, &mut retakes);
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
            if let Some(why) = stall.check(&latest, &stdout_file, watch.events.len(), &watch.events, stopping.is_some()) {
                let v = screenshot(pid, &runs, id, why, (prism::timeline::now_ms() - click_ms) as f64 / 1000.0);
                update(&jobs, id, |j| j.shots.push(v));
            }
            // Hung: while loading, no progress for HUNG_SECS; once settled at the title screen or in a world (where a run
            // can wait or a capture run quietly for minutes) the game's window not responding for that long.
            let in_world = watch.events.last().is_some_and(|e| ["titleScreen", "terrainReady", "leaveDone"].iter().any(|s| e["event"] == *s));
            let frozen = in_world && game_window(pid).is_some_and(super::shot::is_hung);
            if !frozen { frozen_since = None } else if frozen_since.is_none() { frozen_since = Some(Instant::now()) }
            let hung = stopping.is_none() && if in_world {
                frozen_since.is_some_and(|t| t.elapsed() >= Duration::from_secs(HUNG_SECS))
            } else {
                stall.quiet_for() >= Duration::from_secs(HUNG_SECS)
            };
            if hung && kill_now.is_none() {
                let t = (prism::timeline::now_ms() - click_ms) as f64 / 1000.0;
                break corpse(pid, &mut game, &java, &runs, id, &dir, t, if in_world { "window not responding" } else { "no progress" }, &jobs);
            }
            if let Some(p) = playing.lock().expect("playing").take() {
                super::kill_tree(pid);
                let _ = game.wait();
                break format!("{INVALID}: stopped because the user {} ({p})", if exclusive { "came back" } else { "started playing" });
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
            if entered.is_none() && watch.events.iter().any(|e| e["event"] == "inWorld" || e["event"] == "terrainReady") {
                entered = Some(Instant::now());
            }
            if let (Some(t), Some(c)) = (entered, world_cap) {
                if t.elapsed() > Duration::from_secs(c + QUIT_GRACE) {
                    let v = screenshot(pid, &runs, id, "overtime", (prism::timeline::now_ms() - click_ms) as f64 / 1000.0);
                    update(&jobs, id, |j| j.shots.push(v));
                    super::kill_tree(pid);
                    let _ = game.wait();
                    break format!("{OVERTIME}: killed {c} s after reaching the world (the cap for every non-perf run; split longer work into several runs)");
                }
            }
            if hung_on_exit || t0.elapsed() > Duration::from_secs(timeout) {
                let v = screenshot(pid, &runs, id, if hung_on_exit { "hung on exit" } else { "overtime" },
                    (prism::timeline::now_ms() - click_ms) as f64 / 1000.0);
                update(&jobs, id, |j| j.shots.push(v));
                super::kill_tree(pid);
                let _ = game.wait();
                break if hung_on_exit { "killed: still alive 30 s after Stopping!".into() } else { format!("{OVERTIME}: killed at its {timeout} s limit") };
            }
            std::thread::sleep(Duration::from_millis(250));
        };
        watching.store(false, std::sync::atomic::Ordering::Relaxed);
        run_live.store(false, std::sync::atomic::Ordering::Relaxed);
        TEST_PIDS.lock().expect("test pids").remove(&pid);
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(runs.join(format!("{id}.cmd.txt"))) {
            let _ = writeln!(f, "# focus taken back: {} time(s) over {} window(s)", retakes.values().sum::<u32>(), retakes.len());
        }
        let t = prism::timeline::record(&dir, prism::timeline::Session { source: "test".into(), label: lock.clone(), click_ms,
            java_ms: Some(click_ms), window_ms, exit_ms: Some(prism::timeline::now_ms()), ..Default::default() });
        let times: Vec<String> = prism::timeline::phases(&t).iter().filter(|p| !p.idle && p.key != "launcher")
            .map(|p| format!("{} {:.1}s", p.key, p.ms as f64 / 1000.0)).collect();
        let why = if times.is_empty() { why } else { format!("{why}; {}", times.join(", ")) };
        let why = if fps_run { format!("{why}; {}", fps_report(&dir, click_ms.max(0) as u64)) } else { why };
        let why = match &gc_used { Some(g) => format!("{why}; GC flags for this run: {g}"), None => why };
        let crashes = std::fs::read_dir(dir.join("crash-reports")).into_iter().flatten().flatten()
            .filter(|e| e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t >= started)).count();
        Ok(if crashes > 0 { format!("{why}; {crashes} new crash report(s)") } else { why })
    })();
    stitch_jfr(&dir.join("jfr"));
    if let Some(before) = run_set.lock().expect("run options").take() { uncap_undo(&dir.join("options.txt"), &before); }
    if let Some(before) = caps.lock().expect("caps").take() { uncap_undo(&dir.join("options.txt"), &before); }
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
        Ok(d) if d.starts_with("HUNG") => ("hung", d),
        Ok(d) if d.starts_with(INVALID) => ("invalid", d),
        Ok(d) if d.starts_with(OVERTIME) => (OVERTIME, d),
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

/// When a run looks stuck: its log (and output) stopped moving, or it is still loading long after its last stage.
/// One screenshot per stall; the log moving again (or a new stage) arms the next one.
#[derive(Default)]
struct Stall {
    sizes: (u64, u64),
    moved: Option<Instant>,
    stages: usize,
    staged: Option<Instant>,
    quiet_shot: bool,
    slow_shot: bool,
}

/// No new log line for this long counts as a stall.
const QUIET_SECS: u64 = 45;
/// Still loading (no terrainReady yet) this long after the last stage counts too: a game busy in an error loop.
const SLOW_SECS: u64 = 180;

/// No progress at all for this long (no new log or output line, no new stage; in a world: the window not
/// responding) and the run is killed as hung, its state kept for diagnosis (user, 2026-10-30: 90 s, every test).
const HUNG_SECS: u64 = 90;

impl Stall {
    /// How long since the run last showed progress (a log or output line, or a new stage).
    fn quiet_for(&self) -> Duration {
        let since = |t: Option<Instant>| t.map(|t| t.elapsed()).unwrap_or_default();
        since(self.moved).min(since(self.staged))
    }

    fn check(&mut self, log: &Path, out: &Path, stages: usize, events: &[Value], stopping: bool) -> Option<&'static str> {
        let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let sizes = (size(log), size(out));
        let now = Instant::now();
        if sizes != self.sizes || self.moved.is_none() {
            self.sizes = sizes;
            self.moved = Some(now);
            self.quiet_shot = false;
        }
        if stages != self.stages || self.staged.is_none() {
            self.stages = stages;
            self.staged = Some(now);
            self.slow_shot = false;
        }
        if stopping {
            return None;
        }
        if !self.quiet_shot && self.moved.is_some_and(|t| t.elapsed() > Duration::from_secs(QUIET_SECS)) {
            self.quiet_shot = true;
            return Some("its log stopped moving");
        }
        let loaded = events.iter().any(|e| e["event"] == "terrainReady");
        if !loaded && !self.slow_shot && self.staged.is_some_and(|t| t.elapsed() > Duration::from_secs(SLOW_SECS)) {
            self.slow_shot = true;
            return Some("no next stage for 3 minutes");
        }
        None
    }
}

/// The game's main window (the GLFW/LWJGL one when there are several).
fn game_window(pid: u32) -> Option<isize> {
    super::top_windows().into_iter().filter(|w| w.1 == pid && w.2)
        .max_by_key(|w| (w.3.starts_with("GLFW") || w.3.starts_with("LWJGL"), w.0 != 0)).map(|w| w.0)
}

/// A hung run, killed and kept for diagnosis, in this order: screenshot, thread dumps and heap info (jcmd), kill by
/// PID (the whole tree), the log copied beside them. Returns the job's result ("HUNG ...", with every path).
#[allow(clippy::too_many_arguments)]
fn corpse(pid: u32, game: &mut std::process::Child, java: &str, runs: &Path, id: u32, dir: &Path, t: f64, how: &str, jobs: &Shared) -> String {
    let shot = screenshot(pid, runs, id, "hung", t);
    update(jobs, id, |j| j.shots.push(shot.clone()));
    let path = |p: &Path| p.to_string_lossy().replace('\\', "/");
    let jcmd = super::devrun::jcmd_for(java);
    let dumps: Vec<String> = ["threads", "heapinfo"].iter().map(|k| match jcmd.as_ref().map_err(|e| e.clone()).and_then(|j| snapshot(j, pid, runs, k)) {
        Ok(v) => v["file"].as_str().unwrap_or_default().replace('\\', "/"),
        Err(e) => format!("({k}: {e})"),
    }).collect();
    super::kill_tree(pid);
    let _ = game.wait();
    let log = runs.join(format!("{id}-hung-latest.log"));
    let kept = std::fs::copy(dir.join("logs").join("latest.log"), &log).is_ok();
    format!("HUNG ({how} for {HUNG_SECS} s, at {t:.0} s): killed; screenshot {}; threads {}; heap {}; log {}; stdout {}; JFR {}",
        shot["path"].as_str().map(str::to_string).unwrap_or_else(|| format!("({})", shot["error"].as_str().unwrap_or("none"))),
        dumps[0], dumps[1], if kept { path(&log) } else { "(none)".into() }, path(&runs.join(format!("{id}.stdout"))), path(&dir.join("jfr")))
}

/// A screenshot of the game's window into the run folder, as a job record (path, or why there is none).
fn screenshot(pid: u32, runs: &Path, id: u32, why: &str, t: f64) -> Value {
    let t = (t * 10.0).round() / 10.0;
    let mut v = json!({ "t": t, "why": why });
    let Some(h) = game_window(pid) else {
        v["error"] = "the game has no window".into();
        return v;
    };
    let out = runs.join(format!("{id}-stall-{}s.png", t.round() as u64));
    match super::shot::window_png(h, &out) {
        Ok(note) => {
            v["path"] = out.to_string_lossy().replace('\\', "/").into();
            if !note.is_empty() { v["note"] = note.into(); }
        }
        Err(e) => v["error"] = e.into(),
    }
    v
}

const JFR_REPO: &str = "repo-";

/// After a run with a recording: a game that was killed never wrote its recording (or wrote it from an earlier
/// point), but its repository holds every chunk flushed up to its last second. Chunks joined in order are a valid
/// recording, so that becomes obelisk-<stamp>.jfr when it holds more; the repository is then removed.
fn stitch_jfr(jfr: &Path) {
    for repo in std::fs::read_dir(jfr).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(JFR_REPO))) {
        let stamp = repo.file_name().unwrap().to_string_lossy()[JFR_REPO.len()..].to_string();
        let mut chunks = vec![];
        let mut dirs = vec![repo.clone()];
        while let Some(d) = dirs.pop() {
            for p in std::fs::read_dir(&d).into_iter().flatten().flatten().map(|e| e.path()) {
                if p.is_dir() { dirs.push(p) } else if p.extension().is_some_and(|x| x == "jfr") { chunks.push(p) }
            }
        }
        chunks.sort_by_key(|p| p.file_name().map(|n| n.to_os_string()));
        let joined: Vec<u8> = chunks.iter().filter_map(|c| std::fs::read(c).ok()).flat_map(close_chunk).collect();
        let out = jfr.join(format!("obelisk-{stamp}.jfr"));
        let tmp = jfr.join(format!("obelisk-{stamp}.jfr.obelisk-tmp"));
        let have = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        if joined.len() as u64 > have && std::fs::write(&tmp, &joined).is_ok() {
            let _ = std::fs::rename(&tmp, &out);
        }
        let _ = std::fs::remove_dir_all(&repo);
    }
}

/// A chunk the JVM was still writing: its header (rewritten at every flush) gives the size up to the last flush,
/// and its file state (byte 64) stays "being written" until it is set to 0, the state of a finished chunk.
fn close_chunk(mut b: Vec<u8>) -> Vec<u8> {
    if b.len() < 68 || !b.starts_with(b"FLR\0") {
        return vec![];
    }
    let size = u64::from_be_bytes(b[8..16].try_into().unwrap()) as usize;
    if size >= 68 && size < b.len() {
        b.truncate(size);
    }
    b[64] = 0;
    b
}

fn source_file(copy: &str) -> PathBuf {
    tb().root.join("obelisk-runs").join(copy).join("source.json")
}

/// The profile a copy came from (recorded by copy_pack). Copies made before that was recorded are taken to come
/// from the bench's play profile, which is where copies are made from.
fn copy_source(copy: &str) -> Profile {
    std::fs::read(source_file(copy)).ok().and_then(|b| serde_json::from_slice(&b).ok())
        .or_else(|| gate_source(copy))
        .unwrap_or_else(|| play_dir().parent().map(Profile::of_instance).unwrap_or_default())
}

/// gate.py's copies keep no source file. Each notes the pack it was last synced from (gate-sync.json "from", the
/// pack's minecraft folder), so the pool (gate, gate-2, ...) follows whichever pack it mirrors; else, for gate-<slug>,
/// the pack is the instance whose folder name gives that slug (lowercase, every run of other characters one '-'),
/// so its Java and JVM args are used, not the play pack's.
fn gate_source(copy: &str) -> Option<Profile> {
    let from = std::fs::read(tb().root.join(copy).join("gate-sync.json")).ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["from"].as_str().map(PathBuf::from))
        .and_then(|m| m.parent().map(Path::to_path_buf))
        .filter(|inst| inst.join("instance.cfg").is_file());
    if let Some(inst) = from { return Some(Profile::of_instance(&inst)) }
    let slug_of = |name: &str| {
        let mut out = String::new();
        for c in name.to_lowercase().chars() {
            if c.is_ascii_alphanumeric() { out.push(c) } else if !out.ends_with('-') { out.push('-') }
        }
        out.trim_matches('-').to_string()
    };
    let want = copy.strip_prefix("gate-")?;
    let data = prism::data_dir();
    let dir = prism::instances_dir(&data);
    prism::list_instances(&data).into_iter().find(|i| slug_of(&i.id) == want).map(|i| Profile::of_instance(&dir.join(&i.id)))
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
const SWAP_JOURNAL: &str = "swap-journal.json";

/// Runs cut off by an Obelisk restart left their swaps in place (mods set aside, added jars, edited configs): put
/// them back in every copy no game is running in.
fn recover_swaps() {
    for e in std::fs::read_dir(&tb().root).into_iter().flatten().flatten() {
        let copy = e.path();
        let journal = copy.join(".obelisk").join(SWAP_JOURNAL);
        let Ok(bytes) = std::fs::read(&journal) else { continue };
        if game_running_in(&copy) { continue }
        let j: Value = serde_json::from_slice(&bytes).unwrap_or_default();
        let paths = |k: &str| -> Vec<PathBuf> { j[k].as_array().into_iter().flatten().filter_map(|p| p.as_str()).map(PathBuf::from)
            .filter(|p| p.starts_with(&copy)).collect() };
        for p in paths("added") { let _ = std::fs::remove_file(p); }
        for p in paths("off") { let _ = std::fs::rename(&p, p.with_extension("")); }
        for c in j["configs"].as_array().into_iter().flatten() {
            let (Some(path), Some(b)) = (c[0].as_str().map(PathBuf::from), c[1].as_array()) else { continue };
            if !path.starts_with(&copy) { continue }
            let b: Vec<u8> = b.iter().filter_map(|x| x.as_u64().map(|x| x as u8)).collect();
            let _ = std::fs::write(path, b);
        }
        let _ = std::fs::remove_file(&journal);
        audit("swap-recovered", &json!({ "copy": copy }), "put back after a restart");
    }
}

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
        let mut disable = list("disable_mods")?;
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
        // The light test profile (user, KB #143): shaders and Boundless' far terrain off for this run, unless the test
        // is about them ("shaders" / "boundless") or is a perf run, which measures the pack as the user plays it.
        let flag = |k: &str| args.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
        if !flag("perf") && !flag("shaders") {
            for f in ["iris.properties", "oculus.properties"] {
                let path = dir.join("config").join(f);
                let Ok(bytes) = std::fs::read(&path) else { continue };
                let text = String::from_utf8_lossy(&bytes).into_owned();
                if text.lines().any(|l| l.trim() == "enableShaders=true") {
                    configs.push((path, bytes, text.replace("enableShaders=true", "enableShaders=false")));
                }
            }
        }
        if !flag("perf") && !flag("boundless") {
            for e in std::fs::read_dir(&mods).into_iter().flatten().flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                let l = n.to_ascii_lowercase();
                if l.starts_with("boundless") && l.ends_with(".jar") && !disable.contains(&n) { disable.push(n) }
            }
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
        self.journal();
        Ok(())
    }

    /// What apply changed, on disk: if Obelisk restarts mid-run (an update) the next start puts it back.
    fn journal(&self) {
        let Some(copy) = self.mods.parent() else { return };
        let configs: Vec<Value> = self.written.iter().map(|&i| json!([self.configs[i].0, self.configs[i].1])).collect();
        let _ = std::fs::create_dir_all(copy.join(".obelisk"));
        let _ = std::fs::write(copy.join(".obelisk").join(SWAP_JOURNAL),
            serde_json::to_vec(&json!({ "off": self.off, "added": self.added, "configs": configs })).unwrap_or_default());
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
        if bad.is_empty() {
            if let Some(copy) = self.mods.parent() { let _ = std::fs::remove_file(copy.join(".obelisk").join(SWAP_JOURNAL)); }
            Ok(())
        } else { Err(format!("couldn't restore: {bad:?}")) }
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

/// Mods set aside for a perf run (file names start with these).
const PERF_OFF: [&str; 2] = ["dynamic-fps", "absolute-afk-cinema"];
/// Mods that move the camera or slow the window on their own: off for runs whose pictures are the test.
const VISUAL_OFF: [&str; 4] = ["dynamic-fps", "absolute-afk-cinema", "cinematic_respawn", "grandteleport"];

/// The primary monitor's resolution in real pixels (its current display mode, so DPI scaling doesn't shrink it).
fn native_resolution() -> Option<(u32, u32)> {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumDisplaySettingsW(device: *const u16, mode: u32, dm: *mut u8) -> i32;
    }
    // DEVMODEW: dmSize (u16) at 68, dmPelsWidth/Height (u32) at 172/176.
    let mut dm = [0u8; 220];
    dm[68..70].copy_from_slice(&220u16.to_le_bytes());
    if unsafe { EnumDisplaySettingsW(std::ptr::null(), u32::MAX /* ENUM_CURRENT_SETTINGS */, dm.as_mut_ptr()) } == 0 { return None }
    let w = u32::from_le_bytes(dm[172..176].try_into().ok()?);
    let h = u32::from_le_bytes(dm[176..180].try_into().ok()?);
    (w >= 640 && h >= 480).then_some((w, h))
}

/// Out of the user's way but findable (user, 2026-11-06, KB #51; as gate.py's park): minimized without taking the
/// focus, with its normal taskbar button and Win+Tab entry, so the user can look at it or End Task it. A run whose
/// pictures or frame times are the test (`render`) needs a drawn frame instead: beyond every monitor at the bottom of
/// the z-order, still on the taskbar; with `size` (perf) it also loses its frame and takes that size, so the game
/// renders full screen's pixels. Once the user brings a window up it is theirs and left alone.
/// How long after start a test window that takes the focus is always sent back (it's the game, not the user).
const FOCUS_GUARD: Duration = Duration::from_secs(180);
/// Every running test game: one test's window is never "the user's window" to give the focus back to, or two runs
/// hand it to each other forever.
static TEST_PIDS: std::sync::LazyLock<Mutex<std::collections::HashSet<u32>>> = std::sync::LazyLock::new(Default::default);

/// `prev`: the user's last window that isn't this test's, kept current. `guard`: early in the run or the user is in
/// their own game, so a test window in front is never the user opening it: it goes back down and they get theirs back.
fn park_windows(pid: u32, parked: &mut std::collections::HashSet<isize>, render: bool, size: Option<(u32, u32)>, prev: &mut isize, guard: bool,
    retakes: &mut HashMap<isize, u32>) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetWindowPos(h: isize, after: isize, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
        fn SetWindowLongPtrW(h: isize, index: i32, value: isize) -> isize;
        fn GetForegroundWindow() -> isize;
        fn GetWindowRect(h: isize, r: *mut super::shot::Rect) -> i32;
        fn GetWindowLongPtrW(h: isize, index: i32) -> isize;
        fn IsIconic(h: isize) -> i32;
    }
    let fg = unsafe { GetForegroundWindow() };
    let tests = TEST_PIDS.lock().expect("test pids").clone();
    if fg != 0 && !parked.contains(&fg) && !parked.contains(&-fg) && !super::top_windows().iter().any(|w| w.0 == fg && (w.1 == pid || tests.contains(&w.1))) {
        *prev = fg;
    }
    let prev = *prev;
    // Take the focus back from window h, at most MAX_RETAKES times. Minimizing hands it to the next window down (the
    // user's: parked ones sit at the bottom); SetForegroundWindow from a background app is usually refused and flashes
    // a taskbar button instead, so it's only for render windows, which have to stay up.
    // Obelisk never takes the focus back any more (user, 11-11: "repeatedly minimizing a window is not a good idea"):
    // the game is told not to take it (-Dobelisk.background, Optimal), and each window is touched once. This only
    // counts the times a game took it anyway, for the run log.
    let mut retake = |h: isize| { *retakes.entry(h).or_insert(0) += 1; };
    // Checked every time, not once: the game restores and centres its own window when it sets its display mode after
    // start-up, which would leave a full-size window over the user's desktop.
    for (h, p, vis, _, _) in super::top_windows() {
        if p != pid || !vis || parked.contains(&-h) { continue }
        if parked.contains(&h) && fg == h && fg != prev {
            if guard {
                // The game took the focus by itself: keys the user holds would go to it. Down it goes, theirs back.
                retake(h);
                continue;
            }
            parked.insert(-h); // the user opened it: never moved or minimized again
            continue;
        }
        let first = parked.insert(h);
        if first {
            // WS_EX_APPWINDOW, minus TOOLWINDOW and NOACTIVATE: a normal taskbar button and Win+Tab entry.
            unsafe {
                let ex = GetWindowLongPtrW(h, -20 /* GWL_EXSTYLE */);
                SetWindowLongPtrW(h, -20, (ex | 0x0004_0000) & !(0x80 | 0x0800_0000));
            }
        }
        // Every test window is parked off-screen, never minimized: a minimized game stops drawing, and the user
        // watches tests through the taskbar button's live preview (user, 11-11). Moving uses NOACTIVATE: no focus.
        let _ = (first, render);
        // A minimized window draws nothing; moving one back would also look to Windows and window managers like
        // FancyZones as a window dragged back while the user works (user, 2026-11-04).
        if unsafe { IsIconic(h) } != 0 { continue }
        let mut r = super::shot::Rect::default();
        if unsafe { GetWindowRect(h, &mut r) } == 0 { continue }
        let at = r.l == -20000 && r.t == -20000;
        let sized = size.is_none_or(|(w, ht)| r.r - r.l == w as i32 && r.b - r.t == ht as i32);
        if !(at && sized) {
            unsafe {
                match size {
                    Some((w, ht)) => {
                        // WS_POPUP | WS_VISIBLE: no caption or border, so the client area is the whole size.
                        SetWindowLongPtrW(h, -16 /* GWL_STYLE */, (0x8000_0000u32 | 0x1000_0000) as i32 as isize);
                        SetWindowPos(h, 1, -20000, -20000, w as i32, ht as i32, 0x0010 | 0x0020 /* NOACTIVATE | FRAMECHANGED */);
                    }
                    None => { SetWindowPos(h, 1, -20000, -20000, 0, 0, 0x0001 | 0x0010); }
                }
            }
        }
    }
    // The game shows itself focused when it starts: the foreground goes back to what the user had.
    let fg = unsafe { GetForegroundWindow() };
    if fg != prev && parked.contains(&fg) && !parked.contains(&-fg) { retake(fg) }
}

fn foreground_window() -> isize {
    #[link(name = "user32")]
    unsafe extern "system" { fn GetForegroundWindow() -> isize; }
    unsafe { GetForegroundWindow() }
}

// ---------------------------------------------------------------- gate and copies

fn gate(args: &Value, jobs: &Shared) -> Result<Value, String> {
    let owner = label(need(args, "owner")?, "owner")?;
    let mut a: Vec<String> = vec![format!("{}/gate.py", tb().scripts), "run".into(), "--owner".into(), owner.clone()];
    a.extend(["--cycles".into(), num(args, "cycles", 2, 1, 2)?.to_string(), "--hold".into(), num(args, "hold", SMOKE_HOLD, 30, SMOKE_HOLD)?.to_string()]);
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
            // The run limit counts from when the gate holds the lock (benchlock prints "acquired by").
            let mut since: Option<Instant> = None;
            let code = loop {
                if let Some(st) = child.try_wait().map_err(|e| e.to_string())? { break st.code() }
                if since.is_none() && std::fs::read_to_string(&out).is_ok_and(|t| t.contains("acquired by")) { since = Some(Instant::now()) }
                if since.is_some_and(|t| t.elapsed() > Duration::from_secs(RUN_WALL)) {
                    super::kill_tree(child.id());
                    let _ = child.wait();
                    return Ok(format!("{OVERTIME}: killed at the {RUN_WALL} s run limit"));
                }
                std::thread::sleep(Duration::from_secs(2));
            };
            let text = std::fs::read_to_string(&out).unwrap_or_default();
            let verdict = if text.contains("PASS") && code == Some(0) { "PASS" } else { "FAIL" };
            Ok(format!("{verdict} (exit {})", code.map(|c| c.to_string()).unwrap_or("killed".into())))
        })();
        let (state, detail) = match r {
            Ok(d) if d.starts_with("PASS") => ("done", d),
            Ok(d) if d.starts_with(OVERTIME) => (OVERTIME, d),
            Ok(d) => ("failed", d),
            Err(e) => ("failed", e),
        };
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

#[cfg(test)]
mod diag_tests {
    use super::*;
    #[test]
    fn diag_allow_list() {
        let ok = json!({ "diag": ["-XX:+UnlockDiagnosticVMOptions", "-XX:+SafepointTimeout", "-XX:SafepointTimeoutDelay=200",
            "-XX:HandshakeTimeout=500", "-Xlog:safepoint*=info,handshake*=info:file=logs/safepoint.log:uptime,tid"] });
        assert_eq!(diag_flags(&ok).unwrap().len(), 5);
        for bad in ["-XX:+AbortVMOnSafepointTimeout", "-Xlog:gc*:file=x.log", "-Xlog:safepoint:file=../x.log",
            "-Xlog:safepoint:file=C:/x.log", "-XX:SafepointTimeoutDelay=abc"] {
            assert!(diag_flags(&json!({ "diag": [bad] })).is_err(), "{bad}");
        }
    }
}
