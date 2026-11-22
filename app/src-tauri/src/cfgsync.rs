//! Config sync between the play packs (user, 2026-11-17, KB #189). A setting the user changes in a pack's game
//! (options.txt and mod configs under config/, per key) reaches the other packs on its own; a change made while that
//! pack's game was closed (a chat's edit) waits in pending until it is approved. Every write is backed up first and
//! logged in sync.log. A pack whose game is open is never written: its changes wait until it closes. Test copies are
//! never touched. Files are edited in place (only the value text changes), so comments and layout stay.

use obelisk_core as prism;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// The packs kept in step. Test copies live elsewhere and are never synced.
const PACKS: [&str; 3] = ["Odyssey", "Test Dummy", "Foundation"];
const OPTIONS: &str = "options.txt";
/// options.txt keys that belong to one pack: its data version and its own resource packs.
const SKIP: [&str; 3] = ["version", "resourcePacks", "incompatibleResourcePacks"];
/// Mod config formats synced per key. Old Forge .cfg is not (its blocks don't map to keys safely).
const CFG_EXT: [&str; 5] = ["toml", "properties", "ini", "json", "json5"];
/// config/ folders that are a pack's own identity or a tool's, never synced: menus and loading screens, pack-made
/// defaults and loaders, the bench's own mods, and files tools rewrite per copy.
const CFG_SKIP_DIRS: [&str; 12] = ["fancymenu", "drippyloadingscreen", "defaultoptions", "paxi", "openloader", "obelisk",
    "testharness", "speedloader", "crash_assistant", "euphoria_patcher", "bclib", "chunky"];
/// Mod config keys (last part, lowercase) that are a file's bookkeeping, not a setting.
const CFG_SKIP_KEYS: [&str; 7] = ["version", "configversion", "config_version", "schemaversion", "firstrun", "first_run", "lastupdatecheck"];
/// Mod config file names (any part, lowercase) that are a mod's own records, not settings.
const CFG_SKIP_NAMES: [&str; 8] = ["cache", "state", "history", "bookmark", "activity", "stats", "usercache", "-server"];
const KEEP_BACKUPS: usize = 40;
/// Mod config files bigger than this are data (caches, lists), not settings.
const CFG_MAX: u64 = 512 * 1024;

type Opts = BTreeMap<String, String>;

#[derive(Default, Serialize, Deserialize)]
struct State {
    /// Each file's values as of the last sync, by "pack|file": a difference from it is a new change.
    base: BTreeMap<String, Opts>,
    /// Chat changes waiting for approval.
    pending: Vec<Pending>,
    /// Settings to write into a pack's file ("pack|file") once its game is closed.
    outbox: BTreeMap<String, Opts>,
    next_id: u64,
}

fn options() -> String { OPTIONS.into() }

#[derive(Serialize, Deserialize, Clone)]
struct Pending {
    id: u64,
    pack: String,
    #[serde(default = "options")]
    file: String,
    key: String,
    old: Option<String>,
    new: String,
    seen: String,
}

static LOCK: Mutex<()> = Mutex::new(());
/// Per pack: was its game open at the last look.
static OPEN: Mutex<BTreeMap<String, bool>> = Mutex::new(BTreeMap::new());
/// Per "pack|file": the file time last read.
static SEEN: Mutex<BTreeMap<String, SystemTime>> = Mutex::new(BTreeMap::new());

fn dir() -> PathBuf {
    crate::datadir::root().join("config-sync")
}

fn game_dir(pack: &str) -> Option<PathBuf> {
    prism::game_dir(&prism::data_dir(), pack)
}

fn dk(pack: &str, file: &str) -> String { format!("{pack}|{file}") }
fn split(k: &str) -> (&str, &str) { k.split_once('|').unwrap_or((k, OPTIONS)) }

fn load() -> State {
    let mut st: State = std::fs::read(dir().join("state.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    // v1 kept options.txt only, keyed by pack.
    for m in [&mut st.base, &mut st.outbox] {
        let old: Vec<String> = m.keys().filter(|k| !k.contains('|')).cloned().collect();
        for k in old { if let Some(v) = m.remove(&k) { m.insert(dk(&k, OPTIONS), v); } }
    }
    st
}

fn save(st: &State) {
    let _ = std::fs::create_dir_all(dir());
    let tmp = dir().join("state.json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec(st).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, dir().join("state.json"));
    }
}

fn log(msg: &str) {
    use std::io::Write;
    let _ = std::fs::create_dir_all(dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir().join("sync.log")) {
        let _ = writeln!(f, "{} {msg}", crate::control::stamp());
    }
}

/// Every setting in a file: its key and where its value text sits. Unknown or broken files give nothing.
fn scan(file: &str, text: &str) -> Vec<(String, Range<usize>)> {
    let ext = file.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
    let all = if file == OPTIONS { scan_lines(text, ':', false) } else { match ext.as_str() {
        "json" | "json5" => { let mut j = Json { b: text.as_bytes(), i: 0, out: vec![] }; if j.value("").is_some() { j.out } else { vec![] } }
        "toml" | "ini" | "properties" => scan_lines(text, '=', true),
        _ => vec![],
    } };
    // A key that repeats is a list (an order, a set of names), not one setting: left alone.
    let mut n: BTreeMap<&str, usize> = BTreeMap::new();
    for (k, _) in &all { *n.entry(k.as_str()).or_default() += 1 }
    let dup: BTreeSet<String> = n.into_iter().filter(|(_, c)| *c > 1).map(|(k, _)| k.to_string()).collect();
    all.into_iter().filter(|(k, _)| !dup.contains(k)).collect()
}

/// key<sep>value lines (options.txt, .properties) with [sections] (.toml, .ini). Multi-line values and [[array]]
/// tables are skipped: they don't map to one key safely.
fn scan_lines(text: &str, sep: char, sections: bool) -> Vec<(String, Range<usize>)> {
    let mut out = vec![];
    let (mut sec, mut skip_sec, mut open_until): (String, bool, Option<&str>) = (String::new(), false, None);
    let mut at = 0;
    for raw in text.split_inclusive('\n') {
        let start = at;
        at += raw.len();
        let line = raw.trim_end_matches(['\n', '\r']);
        let t = line.trim();
        if let Some(end) = open_until {
            if t.contains(end) || (end == "]" && t.starts_with(']')) { open_until = None }
            continue;
        }
        if t.is_empty() || t.starts_with(['#', ';', '!']) || (sep == '=' && t.starts_with("//")) { continue }
        if sections && t.starts_with('[') {
            skip_sec = t.starts_with("[[");
            sec = t.trim_start_matches('[').split(']').next().unwrap_or_default().trim().to_string();
            continue;
        }
        if skip_sec { continue }
        let Some(p) = line.find(sep) else { continue };
        let key = line[..p].trim().trim_matches(['"', '\'']);
        if key.is_empty() { continue }
        let after = &line[p + 1..];
        let lead = if sep == ':' { 0 } else { after.len() - after.trim_start().len() };
        let val = if sep == ':' { after } else { after.trim() };
        if sections && sep == '=' {
            if val.starts_with("\"\"\"") && val.matches("\"\"\"").count() < 2 { open_until = Some("\"\"\""); continue }
            if val.starts_with("'''") && val.matches("'''").count() < 2 { open_until = Some("'''"); continue }
            if val.starts_with('[') && val.matches('[').count() > val.matches(']').count() { open_until = Some("]"); continue }
        }
        let vs = start + p + 1 + lead;
        let path = if sec.is_empty() { key.to_string() } else { format!("{sec}.{key}") };
        out.push((path, vs..vs + val.len()));
    }
    out
}

/// A forgiving JSON/JSON5 reader that notes where each value sits: scalars and whole arrays are one setting each.
struct Json<'a> { b: &'a [u8], i: usize, out: Vec<(String, Range<usize>)> }

impl Json<'_> {
    fn peek(&self) -> Option<u8> { self.b.get(self.i).copied() }
    fn ws(&mut self) {
        loop {
            while self.peek().is_some_and(|c| c.is_ascii_whitespace()) { self.i += 1 }
            if self.b[self.i..].starts_with(b"//") {
                while self.peek().is_some_and(|c| c != b'\n') { self.i += 1 }
            } else if self.b[self.i..].starts_with(b"/*") {
                self.i += 2;
                while self.i < self.b.len() && !self.b[self.i..].starts_with(b"*/") { self.i += 1 }
                self.i = (self.i + 2).min(self.b.len());
            } else { return }
        }
    }
    fn string(&mut self) -> Option<String> {
        let q = self.peek()?;
        let s = self.i + 1;
        self.i += 1;
        while let Some(c) = self.peek() {
            if c == b'\\' { self.i += 2; continue }
            if c == q { self.i += 1; return Some(String::from_utf8_lossy(&self.b[s..self.i - 1]).into_owned()) }
            self.i += 1;
        }
        None
    }
    fn skip_array(&mut self) -> Option<()> {
        let mut depth = 0;
        loop {
            self.ws();
            match self.peek()? {
                b'"' | b'\'' => { self.string()?; }
                b'[' | b'{' => { depth += 1; self.i += 1 }
                b']' | b'}' => { depth -= 1; self.i += 1; if depth == 0 { return Some(()) } }
                _ => self.i += 1,
            }
        }
    }
    fn value(&mut self, path: &str) -> Option<()> {
        self.ws();
        let s = self.i;
        match self.peek()? {
            b'{' => {
                self.i += 1;
                loop {
                    self.ws();
                    if self.peek()? == b'}' { self.i += 1; return Some(()) }
                    let key = if matches!(self.peek()?, b'"' | b'\'') { self.string()? } else {
                        let k = self.i;
                        while self.peek().is_some_and(|c| c != b':' && !c.is_ascii_whitespace()) { self.i += 1 }
                        String::from_utf8_lossy(&self.b[k..self.i]).into_owned()
                    };
                    self.ws();
                    if self.peek()? != b':' { return None }
                    self.i += 1;
                    self.value(&if path.is_empty() { key } else { format!("{path}/{key}") })?;
                    self.ws();
                    match self.peek()? { b',' => self.i += 1, b'}' => {}, _ => return None }
                }
            }
            b'[' => { self.skip_array()?; }
            b'"' | b'\'' => { self.string()?; }
            _ => {
                while self.peek().is_some_and(|c| !matches!(c, b',' | b'}' | b']' | b'/') && !c.is_ascii_whitespace()) { self.i += 1 }
                if self.i == s { return None }
            }
        }
        if !path.is_empty() { self.out.push((path.to_string(), s..self.i)) }
        Some(())
    }
}

/// A file's settings as key -> value text.
fn parse(file: &str, text: &str) -> Opts {
    scan(file, text).into_iter().filter_map(|(k, r)| text.get(r).map(|v| (k, v.to_string()))).collect()
}

/// Set these keys in the file text, in place, keeping every other byte; new keys go at the end of options.txt
/// (a mod config only gets keys it already has).
fn apply(file: &str, text: &str, set: &Opts) -> String {
    let mut spans: Vec<(Range<usize>, &str)> = vec![];
    let mut done = BTreeSet::new();
    for (k, r) in scan(file, text) {
        if let Some(v) = set.get(&k) { if done.insert(k) { spans.push((r, v)) } }
    }
    spans.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    let mut out = text.to_string();
    for (r, v) in spans { out.replace_range(r, v) }
    if file == OPTIONS {
        if !out.is_empty() && !out.ends_with('\n') { out.push('\n') }
        for (k, v) in set.iter().filter(|(k, _)| !done.contains(k.as_str())) { out.push_str(&format!("{k}:{v}\n")) }
    }
    out
}

fn skip_key(file: &str, key: &str) -> bool {
    if file == OPTIONS { return SKIP.contains(&key) }
    let last = key.rsplit(['.', '/']).next().unwrap_or(key).to_ascii_lowercase();
    CFG_SKIP_KEYS.contains(&last.as_str())
}

/// A pack's mod config files, as paths under its game folder ("config/x/y.toml").
fn configs(game: &Path) -> Vec<String> {
    fn walk(d: &Path, rel: &str, depth: u32, out: &mut Vec<String>) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(ft) = e.file_type() else { continue };
            let r = format!("{rel}/{name}");
            if ft.is_dir() {
                let low = name.to_ascii_lowercase();
                // Per-world folders (JEI bookmarks and history per save) belong to that save.
                if depth < 4 && low != "world" && low != "worlds" && !(depth == 0 && CFG_SKIP_DIRS.contains(&low.as_str())) { walk(&e.path(), &r, depth + 1, out) }
            } else if ft.is_file() {
                let ext = name.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
                let low = name.to_ascii_lowercase();
                let data = CFG_SKIP_NAMES.iter().any(|w| low.contains(w));
                if !data && CFG_EXT.contains(&ext.as_str()) && e.metadata().is_ok_and(|m| m.len() <= CFG_MAX) { out.push(r) }
            }
        }
    }
    let mut out = vec![];
    walk(&game.join("config"), "config", 0, &mut out);
    out
}

/// Copy a pack's file into backups/<time>/<pack>/<file> before it is written; keep the newest KEEP_BACKUPS.
fn backup(pack: &str, rel: &str, file: &Path) -> Result<(), String> {
    let when: String = crate::control::stamp().chars().map(|c| if c.is_ascii_digit() { c } else { '-' }).collect();
    let to = dir().join("backups").join(when).join(pack).join(rel);
    if let Some(p) = to.parent() { std::fs::create_dir_all(p).map_err(|e| e.to_string())? }
    std::fs::copy(file, &to).map_err(|e| format!("backup of {pack} {rel} failed: {e}"))?;
    let mut all: Vec<PathBuf> = std::fs::read_dir(dir().join("backups")).into_iter().flatten().flatten().map(|e| e.path()).collect();
    all.sort();
    if all.len() > KEEP_BACKUPS {
        for old in &all[..all.len() - KEEP_BACKUPS] { let _ = trash::delete(old); }
    }
    Ok(())
}

/// Write waiting settings into every pack whose game is closed.
fn flush(st: &mut State) {
    let mut running: BTreeMap<String, bool> = BTreeMap::new();
    let keys: Vec<String> = st.outbox.keys().cloned().collect();
    for key in keys {
        let (pack, rel) = split(&key);
        let (pack, rel) = (pack.to_string(), rel.to_string());
        let Some(g) = game_dir(&pack) else { st.outbox.remove(&key); continue };
        if *running.entry(pack.clone()).or_insert_with(|| crate::control::game_running(&g)) { continue }
        let file = g.join(&rel);
        let Ok(text) = std::fs::read_to_string(&file) else { st.outbox.remove(&key); continue };
        let set = st.outbox.remove(&key).unwrap_or_default();
        let cur = parse(&rel, &text);
        let set: Opts = set.into_iter().filter(|(k, v)| cur.get(k) != Some(v) && (rel == OPTIONS || cur.contains_key(k))).collect();
        if set.is_empty() { continue }
        if let Err(e) = backup(&pack, &rel, &file) { log(&format!("{pack}: {rel} not written, {e}")); st.outbox.insert(key, set); continue }
        let tmp = file.with_extension("obelisk-sync");
        if std::fs::write(&tmp, apply(&rel, &text, &set)).and_then(|_| std::fs::rename(&tmp, &file)).is_err() {
            log(&format!("{pack}: couldn't write {rel}, will retry"));
            st.outbox.insert(key, set);
            continue;
        }
        let names: Vec<_> = set.keys().cloned().collect();
        st.base.entry(key.clone()).or_default().extend(set);
        if let Ok(t) = std::fs::metadata(&file).and_then(|m| m.modified()) { SEEN.lock().unwrap().insert(key.clone(), t); }
        log(&format!("{pack}: wrote {}{}", if rel == OPTIONS { String::new() } else { format!("{rel}: ") }, names.join(", ")));
    }
}

/// Send these changes in `rel` from `from` to every other pack that has that file.
fn spread(st: &mut State, from: &str, rel: &str, set: &Opts) {
    for p in PACKS.iter().filter(|p| **p != from) {
        let Some(g) = game_dir(p) else { continue };
        if rel != OPTIONS && !g.join(rel).is_file() { continue }
        st.outbox.entry(dk(p, rel)).or_default().extend(set.clone());
    }
}

/// One look at every pack: find new changes, sort them into user (spread now) or chat (pending), write what can be.
pub fn poll() {
    let _g = LOCK.lock().unwrap();
    if dir().join("paused").exists() { return }
    let mut st = load();
    let mut dirty = false;
    // Mod configs are synced when at least two packs have the file.
    let games: Vec<(&str, PathBuf)> = PACKS.iter().filter_map(|p| game_dir(p).map(|g| (*p, g))).collect();
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    let lists: Vec<Vec<String>> = games.iter().map(|(_, g)| configs(g)).collect();
    for l in &lists { for r in l { *count.entry(r.clone()).or_default() += 1 } }
    for ((pack, g), list) in games.iter().zip(lists) {
        let pack = *pack;
        let open = crate::control::game_running(g);
        let was_open = OPEN.lock().unwrap().insert(pack.to_string(), open).unwrap_or(false);
        let mut fresh = 0;
        let files = std::iter::once(OPTIONS.to_string()).chain(list.into_iter().filter(|r| count[r] >= 2));
        for rel in files {
            let key = dk(pack, &rel);
            let file = g.join(&rel);
            let Ok(t) = std::fs::metadata(&file).and_then(|m| m.modified()) else { continue };
            let last = SEEN.lock().unwrap().insert(key.clone(), t);
            if last == Some(t) && st.base.contains_key(&key) { continue }
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            let cur = parse(&rel, &text);
            let Some(base) = st.base.get(&key) else {
                st.base.insert(key, cur);
                fresh += 1;
                dirty = true;
                continue;
            };
            // A mod config key the base lacks is the mod's own addition (an update): taken in quietly.
            let added: Opts = if rel == OPTIONS { Opts::new() } else {
                cur.iter().filter(|(k, _)| !base.contains_key(*k)).map(|(k, v)| (k.clone(), v.clone())).collect()
            };
            let changed: Opts = cur.iter().filter(|(k, v)| !skip_key(&rel, k) && !added.contains_key(*k) && base.get(*k) != Some(*v))
                .map(|(k, v)| (k.clone(), v.clone())).collect();
            if !added.is_empty() { st.base.get_mut(&key).unwrap().extend(added); dirty = true; }
            if changed.is_empty() { continue }
            dirty = true;
            let base = st.base.get(&key).cloned().unwrap_or_default();
            let what = if rel == OPTIONS { String::new() } else { format!("{rel}: ") };
            // The user's own edit: made in that pack's game (open now or at the last look).
            if open || was_open {
                let olds: Vec<String> = changed.iter().map(|(k, v)| format!("{k} {} -> {v}", base.get(k).map(String::as_str).unwrap_or("(none)"))).collect();
                log(&format!("{pack}: user changed {what}{}; syncing to the other packs", olds.join(", ")));
                st.pending.retain(|p| !(p.pack == pack && p.file == rel && changed.contains_key(&p.key)));
                st.base.get_mut(&key).unwrap().extend(changed.clone());
                spread(&mut st, pack, &rel, &changed);
            } else {
                for (k, v) in &changed {
                    if st.pending.iter().any(|p| p.pack == pack && p.file == rel && p.key == *k && p.new == *v) { continue }
                    st.pending.retain(|p| !(p.pack == pack && p.file == rel && p.key == *k));
                    st.next_id += 1;
                    let old = base.get(k).cloned();
                    log(&format!("{pack}: changed with its game closed (a chat?): {what}{k} {} -> {v}; waiting for approval (#{})",
                        old.as_deref().unwrap_or("(none)"), st.next_id));
                    st.pending.push(Pending { id: st.next_id, pack: pack.into(), file: rel.clone(), key: k.clone(), old, new: v.clone(), seen: crate::control::stamp() });
                }
            }
        }
        if fresh > 0 { log(&format!("{pack}: baseline taken for {fresh} file(s)")) }
    }
    if !st.outbox.is_empty() { flush(&mut st); dirty = true; }
    if dirty { save(&st) }
}

/// Start the background look (every 20 s). Only on this PC's bench setup (the packs are this user's own; another
/// player with a pack named "Foundation" must never get its settings rewritten), and idle without any of the packs.
pub fn start() {
    if !crate::control::has_bench() { return }
    std::thread::spawn(|| loop {
        if PACKS.iter().any(|p| game_dir(p).is_some()) { poll() }
        std::thread::sleep(Duration::from_secs(20));
    });
}

fn tail(n: usize) -> Vec<String> {
    let t = std::fs::read_to_string(dir().join("sync.log")).unwrap_or_default();
    let v: Vec<String> = t.lines().map(String::from).collect();
    v[v.len().saturating_sub(n)..].to_vec()
}

/// The config_sync tool: status (default), approve / reject (id, or all=true), sync (look now).
pub fn call(args: &Value) -> Result<Value, String> {
    if !crate::control::has_bench() { return Err("config sync runs only on the test bench PC (bench.json)".into()) }
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("status");
    if action == "sync" { poll() }
    if matches!(action, "approve" | "reject") {
        let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
        let id = args.get("id").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim_start_matches('#').parse().ok())));
        if !all && id.is_none() { return Err("pass id=<pending id> or all=true".into()) }
        let _g = LOCK.lock().unwrap();
        let mut st = load();
        let (pick, keep): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut st.pending).into_iter().partition(|p| all || Some(p.id) == id);
        st.pending = keep;
        if pick.is_empty() { return Err("no such pending change".into()) }
        for p in &pick {
            st.base.entry(dk(&p.pack, &p.file)).or_default().insert(p.key.clone(), p.new.clone());
            let what = if p.file == OPTIONS { String::new() } else { format!("{}: ", p.file) };
            if action == "approve" {
                spread(&mut st, &p.pack, &p.file, &Opts::from([(p.key.clone(), p.new.clone())]));
                log(&format!("{}: approved #{} {what}{} = {}; syncing to the other packs", p.pack, p.id, p.key, p.new));
            } else {
                log(&format!("{}: rejected #{} {what}{} = {}; stays in that pack only", p.pack, p.id, p.key, p.new));
            }
        }
        flush(&mut st);
        save(&st);
    } else if action != "status" && action != "sync" {
        return Err("action: status, approve, reject or sync".into());
    }
    let st = load();
    let files = st.base.keys().filter(|k| split(k).1 != OPTIONS).count();
    Ok(json!({
        "packs": PACKS.iter().filter(|p| game_dir(p).is_some()).collect::<Vec<_>>(),
        "paused": dir().join("paused").exists(),
        "mod_config_files_tracked": files,
        "pending": st.pending.iter().map(|p| json!({ "id": p.id, "pack": p.pack, "file": p.file, "key": p.key, "old": p.old, "new": p.new, "seen": p.seen })).collect::<Vec<_>>(),
        "waiting_for_game_to_close": st.outbox,
        "log": tail(15),
        "folder": dir(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn apply_keeps_order_and_adds_new() {
        let t = "version:3955\nfov:0.0\nkey_key.attack:key.mouse.left\n";
        let set = Opts::from([("fov".into(), "1.0".into()), ("gamma".into(), "0.5".into())]);
        assert_eq!(apply(OPTIONS, t, &set), "version:3955\nfov:1.0\nkey_key.attack:key.mouse.left\ngamma:0.5\n");
        assert_eq!(parse(OPTIONS, t)["key_key.attack"], "key.mouse.left");
    }
    #[test]
    fn toml_in_place() {
        let t = "#c\n[client]\n\t#note\n\tshowHud = true\n\tlist = [\"a\",\n\t\"b\"]\n\tname = \"x\" # tail\n[[arr]]\n\tk = 1\n";
        let p = parse("config/m.toml", t);
        assert_eq!(p["client.showHud"], "true");
        assert_eq!(p["client.name"], "\"x\" # tail");
        assert!(!p.contains_key("client.list") && !p.contains_key("arr.k"));
        let out = apply("config/m.toml", t, &Opts::from([("client.showHud".into(), "false".into()), ("client.nope".into(), "1".into())]));
        assert_eq!(out, t.replace("showHud = true", "showHud = false"));
    }
    #[test]
    fn json_in_place() {
        let t = "{\n  // c\n  \"a\": {\"b\": 1, \"c\": [1, {\"x\": 2}]},\n  d: 'q',\n  \"e\": true,\n}\n";
        let p = parse("config/m.json5", t);
        assert_eq!(p["a/b"], "1");
        assert_eq!(p["a/c"], "[1, {\"x\": 2}]");
        assert_eq!(p["d"], "'q'");
        assert_eq!(p["e"], "true");
        let out = apply("config/m.json5", t, &Opts::from([("a/b".into(), "5".into()), ("e".into(), "false".into())]));
        assert_eq!(out, t.replace("\"b\": 1", "\"b\": 5").replace("\"e\": true", "\"e\": false"));
    }
    #[test]
    fn properties() {
        let t = "# x\nfoo=bar\nbaz = 2\n";
        let p = parse("config/m.properties", t);
        assert_eq!(p["foo"], "bar");
        assert_eq!(p["baz"], "2");
    }
}

#[cfg(test)]
mod real {
    use super::*;
    /// Round trip over the real packs' configs: parse, write every value back, text must be unchanged.
    #[test]
    #[ignore]
    fn real_packs_round_trip() {
        let base = std::path::Path::new("D:/Modding/Minecraft/Obelisk/Instances");
        let (mut files, mut keys, mut empty, mut bad) = (0, 0, vec![], vec![]);
        for p in PACKS {
            let g = base.join(p).join("minecraft");
            for rel in configs(&g).into_iter().chain([OPTIONS.to_string()]) {
                let Ok(t) = std::fs::read_to_string(g.join(&rel)) else { continue };
                let o = parse(&rel, &t);
                files += 1; keys += o.len();
                if o.is_empty() && !t.trim().is_empty() { empty.push(format!("{p}/{rel}")) }
                if apply(&rel, &t, &o) != t { bad.push(format!("{p}/{rel}")) }
            }
        }
        eprintln!("files {files} keys {keys}; no keys found in {}:", empty.len());
        for e in empty.iter().take(40) { eprintln!("  {e}") }
        eprintln!("round trip changed {}: {bad:?}", bad.len());
        assert!(bad.is_empty());
    }
}
