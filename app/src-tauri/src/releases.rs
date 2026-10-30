//! Release lines for the mods built on this PC (user rule 00, 2026-10-30).
//!
//! A chat submits a build as a fix or a feature. Fixes are stable and go into every pack that has the mod;
//! features are test builds and go into TEST DUMMY only, until promoted (the user played it, or three clean
//! TEST DUMMY sessions). Each install waits for its pack to be closed, passes a 2-cycle gate in that pack's own
//! copy first, takes a safety backup and keeps the old jar, and is rolled back when a later crash in that pack
//! names the new build as the code that was running. Play packs are never launched: gates run in copies.

use super::control as c;
use obelisk_core as prism;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// TEST DUMMY's folder in the instances folder.
const TEST_PACK: &str = "Foundation (EXPERIMENTAL)";
/// Clean TEST DUMMY sessions that prove a test build.
const PROVEN_SESSIONS: usize = 3;
/// How long after an install a crash naming the build rolls it back.
const WATCH_SECS: u64 = 14 * 86_400;
/// Tries to start a gate (the copy busy, the lock held) before a rollout counts as failed.
const MAX_ATTEMPTS: u32 = 12;

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
struct Build {
    mod_id: String,
    version: String,
    file: String,
    sha1: String,
    /// "fix" or "feature".
    kind: String,
    /// "stable" or "test".
    channel: String,
    owner: String,
    notes: String,
    submitted: u64,
    /// How a test build became stable, e.g. "user-played 2026-10-30".
    promoted: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
struct Rollout {
    id: u64,
    mod_id: String,
    version: String,
    /// The pack's folder name.
    pack: String,
    /// queued, gating, installed, failed, skipped, superseded, rolled-back.
    state: String,
    detail: String,
    updated: u64,
    retry_at: u64,
    attempts: u32,
    /// Passed its gate already (it only waits for the pack to close).
    gated: bool,
    prev_file: String,
    backup: String,
    log: String,
    installed_at: u64,
    crashes_seen: Vec<String>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Registry {
    builds: Vec<Build>,
    rollouts: Vec<Rollout>,
    /// Packs that take no automatic releases.
    opt_out: Vec<String>,
    next_id: u64,
}

static LOCK: Mutex<()> = Mutex::new(());

fn root() -> PathBuf {
    c::bench_root().join("releases")
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn today() -> String {
    c::stamp()[..10].to_string()
}

fn load() -> Registry {
    std::fs::read(root().join("registry.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Read, change and save the registry under one lock.
fn with<T>(f: impl FnOnce(&mut Registry) -> Result<T, String>) -> Result<T, String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = load();
    let out = f(&mut r)?;
    std::fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    let tmp = root().join("registry.json.obelisk-tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&r).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, root().join("registry.json")).map_err(|e| e.to_string())?;
    Ok(out)
}

fn jar_path(b: &Build) -> PathBuf {
    root().join(&b.mod_id).join(&b.version).join(&b.file)
}

fn sha_of(p: &Path) -> String {
    std::fs::read(p).map(|b| prism::sha1_hex(&b)).unwrap_or_default()
}

struct Pack {
    id: String,
    name: String,
    game: PathBuf,
}

fn packs() -> Vec<Pack> {
    let data = prism::data_dir();
    let dir = prism::instances_dir(&data);
    prism::list_instances(&data).into_iter().filter_map(|i| {
        let base = dir.join(&i.id);
        let game = [base.join("minecraft"), base.join(".minecraft")].into_iter().find(|g| g.join("mods").is_dir())?;
        Some(Pack { id: i.id, name: i.name, game })
    }).collect()
}

fn pack(id: &str) -> Result<Pack, String> {
    packs().into_iter().find(|p| p.id == id).ok_or_else(|| format!("no pack called {id}"))
}

/// The enabled jar in a pack that provides this mod id.
fn installed_file(game: &Path, mod_id: &str) -> Option<String> {
    prism::dupes::index(&game.join("mods")).remove(mod_id).and_then(|v| v.into_iter().next())
}

/// The settings panel: status and the user's own actions (submitting stays with the chats that build).
pub fn from_ui(cmd: &str, args: &Value) -> Result<Value, String> {
    if !c::bench_ready() {
        return Err("no test bench on this PC".into());
    }
    match cmd {
        "release-status" | "release-promote" | "release-rollback" | "release-optout" => command(cmd, args),
        _ => Err(format!("{cmd} isn't available here")),
    }
}

pub fn command(cmd: &str, args: &Value) -> Result<Value, String> {
    match cmd {
        "release-submit" => submit(args),
        "release-status" => Ok(status()),
        "release-promote" => promote(args),
        "release-rollback" => {
            let m = c::arg(args, "mod")?;
            let p = c::arg(args, "pack")?;
            let id = with(|r| r.rollouts.iter().rev().find(|o| o.mod_id == m && o.pack == p && o.state == "installed")
                .map(|o| o.id).ok_or_else(|| format!("no installed release of {m} in {p}")))?;
            rollback(id, &format!("rolled back on request: {}", c::opt(args, "reason").unwrap_or("no reason given")))
        }
        "release-optout" => {
            let p = pack(c::arg(args, "pack")?)?;
            let on = args.get("on").and_then(|v| v.as_bool()).unwrap_or(true);
            with(|r| {
                r.opt_out.retain(|x| *x != p.id);
                if on { r.opt_out.push(p.id.clone()) }
                Ok(json!({ "optOut": r.opt_out }))
            })
        }
        _ => Err(format!("unknown command {cmd}")),
    }
}

fn submit(args: &Value) -> Result<Value, String> {
    let jar = c::bench_jar(c::arg(args, "jar")?)?;
    let kind = c::arg(args, "kind")?;
    if !matches!(kind, "fix" | "feature") {
        return Err("\"kind\" is fix (a bug fix: stable, goes into every pack) or feature (a test build: TEST DUMMY only)".into());
    }
    let owner = c::arg(args, "owner")?.chars().take(60).collect::<String>();
    let notes = c::opt(args, "notes").unwrap_or("").chars().take(500).collect::<String>();
    let ids = prism::dupes::own_ids(&jar);
    let mod_id = match c::opt(args, "mod") {
        Some(m) if ids.iter().any(|i| i == m) => m.to_string(),
        Some(m) => return Err(format!("the jar doesn't declare the mod id {m} (it declares {})", ids.join(", "))),
        None if ids.len() == 1 => ids[0].clone(),
        None => return Err(format!("the jar declares {}; pass \"mod\"", if ids.is_empty() { "no mod id".into() } else { ids.join(", ") })),
    };
    let version = c::opt(args, "version").map(str::to_string).or_else(|| prism::jar_version(&jar))
        .ok_or("couldn't read the jar's version; pass \"version\"")?;
    if version.is_empty() || version.len() > 60 || !version.chars().all(|ch| ch.is_ascii_alphanumeric() || ".-+_".contains(ch)) {
        return Err(format!("version {version:?}: use letters, digits and . - + _ only"));
    }
    let file = jar.file_name().map(|f| f.to_string_lossy().into_owned()).ok_or("bad jar path")?;
    let bytes = std::fs::read(&jar).map_err(|e| e.to_string())?;
    let sha1 = prism::sha1_hex(&bytes);
    with(|r| {
        if let Some(b) = r.builds.iter().find(|b| b.mod_id == mod_id && b.version == version) {
            return Err(if b.sha1 == sha1 { format!("{mod_id} {version} is already submitted ({} line)", b.channel) }
                else { format!("{mod_id} {version} was already submitted with different contents; give this build a new version") });
        }
        let b = Build { mod_id: mod_id.clone(), version: version.clone(), file, sha1, kind: kind.into(),
            channel: if kind == "fix" { "stable" } else { "test" }.into(), owner, notes, submitted: now(), promoted: String::new() };
        let dest = jar_path(&b);
        std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;
        r.builds.push(b.clone());
        let targets = enqueue(r, &b);
        Ok(json!({ "mod": b.mod_id, "version": b.version, "channel": b.channel, "targets": targets,
            "note": "Each pack is gated (2 cycles, in its own copy) and installed one at a time, after the pack is closed. Follow it with release_status." }))
    })
}

/// Queue a build for every pack its line reaches.
fn enqueue(r: &mut Registry, b: &Build) -> Vec<Value> {
    let mut out = vec![];
    for p in packs() {
        if b.channel == "test" && p.id != TEST_PACK {
            continue;
        }
        let Some(cur) = installed_file(&p.game, &b.mod_id) else { continue };
        let cur_sha = sha_of(&p.game.join("mods").join(&cur));
        if cur_sha == b.sha1 {
            out.push(json!({ "pack": p.name, "state": "installed", "detail": "already has this build" }));
            continue;
        }
        let ahead = r.builds.iter().find(|x| x.mod_id == b.mod_id && x.sha1 == cur_sha && x.channel == "test");
        let (state, detail) = if r.opt_out.contains(&p.id) {
            ("skipped", "this pack takes no automatic releases".to_string())
        } else if let (true, Some(t)) = (b.channel == "stable", ahead) {
            ("skipped", format!("has test build {}; port the fix into the test line too", t.version))
        } else {
            ("queued", String::new())
        };
        for o in r.rollouts.iter_mut().filter(|o| o.mod_id == b.mod_id && o.pack == p.id && o.state == "queued") {
            o.state = "superseded".into();
            o.detail = format!("replaced by {}", b.version);
        }
        r.next_id += 1;
        r.rollouts.push(Rollout { id: r.next_id, mod_id: b.mod_id.clone(), version: b.version.clone(), pack: p.id.clone(),
            state: state.into(), detail: detail.clone(), updated: now(), ..Default::default() });
        out.push(json!({ "pack": p.name, "state": state, "detail": detail }));
    }
    out
}

fn promote(args: &Value) -> Result<Value, String> {
    let m = c::arg(args, "mod")?;
    let v = c::arg(args, "version")?;
    let reason = c::arg(args, "reason")?;
    if !matches!(reason, "user-played" | "proven-in-td") {
        return Err("\"reason\" is user-played (the user played it and it works) or proven-in-td (3 clean TEST DUMMY sessions; checked)".into());
    }
    with(|r| {
        let i = r.builds.iter().position(|b| b.mod_id == m && b.version == v).ok_or_else(|| format!("no build {m} {v}"))?;
        if r.builds[i].channel != "test" {
            return Err(format!("{m} {v} is already stable"));
        }
        let evidence = if reason == "proven-in-td" { proven(r, &r.builds[i])? } else { "the user played it".into() };
        r.builds[i].channel = "stable".into();
        r.builds[i].promoted = format!("{reason} {}", today());
        let b = r.builds[i].clone();
        let targets = enqueue(r, &b);
        Ok(json!({ "mod": m, "version": v, "channel": "stable", "evidence": evidence, "targets": targets }))
    })
}

/// Three TEST DUMMY sessions since the build went in, and no crash there that names it.
fn proven(r: &Registry, b: &Build) -> Result<String, String> {
    let o = r.rollouts.iter().rev().find(|o| o.mod_id == b.mod_id && o.version == b.version && o.pack == TEST_PACK && o.state == "installed")
        .ok_or("this build isn't installed in TEST DUMMY")?;
    let p = pack(TEST_PACK)?;
    let since = std::time::UNIX_EPOCH + Duration::from_secs(o.installed_at);
    // Each session leaves one log: latest.log while it's the newest, then a dated .log.gz.
    let sessions = std::fs::read_dir(p.game.join("logs")).into_iter().flatten().flatten().filter(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        (n == "latest.log" || n.ends_with(".log.gz")) && e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t > since)
    }).count();
    if let Some(name) = crash_naming(&p.game, b, o.installed_at).into_iter().next() {
        return Err(format!("{name} in TEST DUMMY names this build as what was running"));
    }
    if sessions < PROVEN_SESSIONS {
        return Err(format!("{sessions} TEST DUMMY session(s) since it went in; {PROVEN_SESSIONS} are needed"));
    }
    Ok(format!("{sessions} TEST DUMMY sessions since it went in, no crash naming it"))
}

/// Crash reports in a pack since `after` whose diagnosis says this mod's code was running.
fn crash_naming(game: &Path, b: &Build, after: u64) -> Vec<String> {
    prism::crash::list(game).into_iter().filter(|rep| rep.kind == "crash" && rep.time_ms / 1000 > after).filter_map(|rep| {
        let d = prism::crash::diagnose(game, &rep.file).ok()?;
        d.suspects.first().filter(|s| s.mod_id == b.mod_id && prism::crash::ran(s)).map(|_| rep.file.clone())
    }).collect()
}

pub fn start() {
    std::thread::spawn(|| loop {
        if c::bench_ready() {
            if let Err(e) = tick() {
                log(&format!("release worker: {e}"));
                super::errlog::record("releases", &e);
            }
        }
        std::thread::sleep(Duration::from_secs(60));
    });
}

fn log(line: &str) {
    use std::io::Write;
    let _ = std::fs::create_dir_all(root());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(root().join("releases.log")) {
        let _ = writeln!(f, "{} {line}", c::stamp());
    }
}

fn tick() -> Result<(), String> {
    watch_crashes();
    let next = with(|r| {
        let t = now();
        let kind = |o: &Rollout| r.builds.iter().find(|b| b.mod_id == o.mod_id && b.version == o.version).map(|b| b.kind.clone()).unwrap_or_default();
        let mut q: Vec<&Rollout> = r.rollouts.iter().filter(|o| o.state == "queued" && o.retry_at <= t).collect();
        // Fixes go first, then oldest first.
        q.sort_by_key(|o| (kind(o) != "fix", o.id));
        Ok(q.first().map(|o| (*o).clone()))
    })?;
    match next { Some(o) => run(o), None => Ok(()) }
}

fn set(id: u64, state: &str, detail: &str, f: impl FnOnce(&mut Rollout)) -> Result<(), String> {
    with(|r| {
        let o = r.rollouts.iter_mut().find(|o| o.id == id).ok_or("rollout gone")?;
        o.state = state.into();
        o.detail = detail.into();
        o.updated = now();
        f(o);
        Ok(())
    })
}

fn later(id: u64, detail: &str, secs: u64) -> Result<(), String> {
    set(id, "queued", detail, |o| o.retry_at = now() + secs)
}

fn run(o: Rollout) -> Result<(), String> {
    let b = load().builds.into_iter().find(|b| b.mod_id == o.mod_id && b.version == o.version).ok_or("build gone")?;
    let p = match pack(&o.pack) { Ok(p) => p, Err(e) => return set(o.id, "skipped", &e, |_| {}) };
    if c::game_running(&p.game) {
        return later(o.id, "waiting for the pack to close", 300);
    }
    let Some(cur) = installed_file(&p.game, &b.mod_id) else { return set(o.id, "skipped", "the pack no longer has this mod", |_| {}) };
    if sha_of(&p.game.join("mods").join(&cur)) == b.sha1 {
        return set(o.id, "installed", "already has this build", |o| o.installed_at = now());
    }
    if !o.gated {
        set(o.id, "gating", "2-cycle gate in a copy of the pack", |o| o.attempts += 1)?;
        let logs = root().join("logs");
        std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
        let log_file = logs.join(format!("{}-{}-{}.txt", o.id, b.mod_id, b.version));
        let out = std::fs::File::create(&log_file).map_err(|e| e.to_string())?;
        // The old jar's name as a glob that matches only itself.
        let glob = cur.replace('[', "[[]");
        let status = c::python().args([c::gate_script().as_str(), "run", "--owner", "Obelisk releases", "--pack", &p.id, "--sync",
            "--cycles", "2", "--swap", &jar_path(&b).to_string_lossy(), "--replace", &glob, "--mods", &b.mod_id,
            "--tag", &format!("rel{}-{}", o.id, b.mod_id)])
            .current_dir(c::bench_root()).stdout(out.try_clone().map_err(|e| e.to_string())?).stderr(out)
            .status().map_err(|e| e.to_string())?;
        let text = std::fs::read_to_string(&log_file).unwrap_or_default();
        let ran = text.contains("PASS") || text.contains("FAIL");
        let lf = log_file.to_string_lossy().into_owned();
        if !ran {
            let why = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("the gate didn't start").to_string();
            if o.attempts + 1 >= MAX_ATTEMPTS {
                notice(&b, &p, &format!("gate couldn't start after {MAX_ATTEMPTS} tries: {why}"));
                return set(o.id, "failed", &format!("the gate couldn't start: {why}"), |o| o.log = lf);
            }
            return set(o.id, "queued", &format!("waiting to gate: {why}"), |o| { o.retry_at = now() + 900; o.log = lf });
        }
        if !(text.contains("PASS") && status.success()) {
            notice(&b, &p, &format!("gate FAILED; not installed. Log: {lf}"));
            return set(o.id, "failed", "the gate failed, so it wasn't installed", |o| o.log = lf);
        }
        set(o.id, "gating", "gate passed", |o| { o.gated = true; o.log = lf })?;
        if c::game_running(&p.game) {
            return later(o.id, "gate passed; waiting for the pack to close", 300);
        }
    }
    install(&o, &b, &p, &cur)
}

fn install(o: &Rollout, b: &Build, p: &Pack, cur: &str) -> Result<(), String> {
    let mods = p.game.join("mods");
    prism::history::create(&p.game, "safety", &format!("Before {} {} ({} release)", b.mod_id, b.version, b.channel), true)?;
    let keep = p.game.join("mods-removed-releases");
    std::fs::create_dir_all(&keep).map_err(|e| e.to_string())?;
    let backup = keep.join(format!("{}-{cur}", now()));
    std::fs::copy(mods.join(cur), &backup).map_err(|e| e.to_string())?;
    let tmp = mods.join(format!("{}.obelisk-tmp", b.file));
    std::fs::copy(jar_path(b), &tmp).map_err(|e| e.to_string())?;
    if sha_of(&tmp) != b.sha1 {
        let _ = std::fs::remove_file(&tmp);
        return set(o.id, "failed", "the stored build doesn't match its checksum", |_| {});
    }
    if b.file != cur {
        std::fs::remove_file(mods.join(cur)).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, mods.join(&b.file)).map_err(|e| e.to_string())?;
    let _ = prism::history::create(&p.game, "safety", &format!("{} {} installed ({} release)", b.mod_id, b.version, b.channel), true);
    queue_line(&format!("{} {} ({} line, {}) in {}: gate PASS, installed by Obelisk releases. Old jar kept in {}. Check: {}",
        b.mod_id, b.version, b.channel, b.kind, p.name, backup.display(), if b.notes.is_empty() { "it works as before" } else { &b.notes }));
    log(&format!("installed {} {} in {}", b.mod_id, b.version, p.id));
    set(o.id, "installed", &format!("installed {}", today()), |o| {
        o.installed_at = now();
        o.prev_file = cur.into();
        o.backup = backup.to_string_lossy().into_owned();
    })
}

fn rollback(id: u64, reason: &str) -> Result<Value, String> {
    let r = load();
    let o = r.rollouts.iter().find(|o| o.id == id).cloned().ok_or("rollout gone")?;
    let b = r.builds.iter().find(|b| b.mod_id == o.mod_id && b.version == o.version).cloned().ok_or("build gone")?;
    let p = pack(&o.pack)?;
    if c::game_running(&p.game) {
        return Err(format!("{} is open; roll back after it closes", p.name));
    }
    let mods = p.game.join("mods");
    let now_file = mods.join(&b.file);
    if sha_of(&now_file) != b.sha1 {
        return Err(format!("{} in {} has changed since the release went in; not touching it", b.file, p.name));
    }
    if !Path::new(&o.backup).is_file() {
        return Err(format!("the old jar's backup ({}) is missing", o.backup));
    }
    prism::history::create(&p.game, "safety", &format!("Before rolling back {} {}", b.mod_id, b.version), true)?;
    std::fs::copy(&o.backup, mods.join(&o.prev_file)).map_err(|e| e.to_string())?;
    if b.file != o.prev_file {
        std::fs::remove_file(&now_file).map_err(|e| e.to_string())?;
    }
    let _ = prism::history::create(&p.game, "safety", &format!("Rolled back {} {}", b.mod_id, b.version), true);
    notice(&b, &p, &format!("rolled back: {reason}"));
    queue_line(&format!("{} {} ROLLED BACK in {} ({reason}); {} is back.", b.mod_id, b.version, p.name, o.prev_file));
    log(&format!("rolled back {} {} in {}: {reason}", b.mod_id, b.version, p.id));
    set(id, "rolled-back", reason, |_| {})?;
    Ok(json!({ "rolledBack": true, "pack": p.name, "restored": o.prev_file }))
}

/// Roll back a fresh install when a crash in its pack names it as the code that was running.
fn watch_crashes() {
    let t = now();
    let r = load();
    for o in r.rollouts.iter().filter(|o| o.state == "installed" && o.installed_at > 0 && o.installed_at + WATCH_SECS > t) {
        let Some(b) = r.builds.iter().find(|b| b.mod_id == o.mod_id && b.version == o.version) else { continue };
        let Ok(p) = pack(&o.pack) else { continue };
        let fresh: Vec<String> = crash_naming(&p.game, b, o.installed_at).into_iter().filter(|f| !o.crashes_seen.contains(f)).collect();
        let Some(first) = fresh.first() else { continue };
        if c::game_running(&p.game) {
            continue;
        }
        let reason = format!("{first} names {} {} as the code that was running", b.mod_id, b.version);
        match rollback(o.id, &reason) {
            Ok(_) => {}
            Err(e) => {
                log(&format!("auto rollback of {} in {} not done: {e}", b.mod_id, p.id));
                let _ = with(|r| {
                    if let Some(x) = r.rollouts.iter_mut().find(|x| x.id == o.id) { x.crashes_seen.extend(fresh.clone()) }
                    Ok(())
                });
            }
        }
    }
}

/// A line for the build's owner in releases/<mod>/NOTICES.md (owners read it, and release_status shows it).
fn notice(b: &Build, p: &Pack, what: &str) {
    use std::io::Write;
    let dir = root().join(&b.mod_id);
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("NOTICES.md")) {
        let _ = writeln!(f, "- {} {} {} in {} ({}): {what}", &c::stamp()[..16], b.mod_id, b.version, p.name, b.owner);
    }
}

fn queue_line(text: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(c::bench_root().join("PLAYTEST-QUEUE.md")) {
        let _ = writeln!(f, "- {} [Obelisk releases] {text}", today());
    }
}

fn status() -> Value {
    let r = load();
    let ids: std::collections::BTreeSet<&str> = r.builds.iter().map(|b| b.mod_id.as_str()).collect();
    let mut per_pack = vec![];
    for p in packs() {
        let index = prism::dupes::index(&p.game.join("mods"));
        let mut mods = serde_json::Map::new();
        for id in &ids {
            let Some(file) = index.get(*id).and_then(|v| v.first()) else { continue };
            let sha = sha_of(&p.game.join("mods").join(file));
            let known = r.builds.iter().find(|b| b.mod_id == *id && b.sha1 == sha);
            mods.insert(id.to_string(), json!({ "file": file, "version": known.map(|b| b.version.clone()),
                "line": known.map(|b| b.channel.clone()).unwrap_or_else(|| "unregistered".into()) }));
        }
        per_pack.push(json!({ "pack": p.id, "name": p.name, "optOut": r.opt_out.contains(&p.id), "mods": mods }));
    }
    let lines: Vec<Value> = ids.iter().map(|id| {
        let latest = |ch: &str| r.builds.iter().rev().find(|b| b.mod_id == *id && b.channel == ch).map(|b| json!({ "version": b.version,
            "kind": b.kind, "owner": b.owner, "notes": b.notes, "promoted": b.promoted }));
        let notices = std::fs::read_to_string(root().join(id).join("NOTICES.md")).unwrap_or_default();
        json!({ "mod": id, "stable": latest("stable"), "test": latest("test"),
            "notices": notices.lines().rev().take(5).collect::<Vec<_>>() })
    }).collect();
    let queue: Vec<&Rollout> = r.rollouts.iter().filter(|o| matches!(o.state.as_str(), "queued" | "gating")).collect();
    let recent: Vec<&Rollout> = r.rollouts.iter().rev().filter(|o| !matches!(o.state.as_str(), "queued" | "gating")).take(30).collect();
    json!({ "mods": lines, "packs": per_pack, "queue": queue, "recent": recent, "testPack": TEST_PACK })
}
