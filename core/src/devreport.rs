//! Tester reports: a hidden Developer settings tab (unlocked with a password only its hash is kept of) lets a pack
//! member's Obelisk send its crash reports, game logs, mod list and Obelisk's own errors to that pack's owner, over
//! the pack's own encrypted connection (share_net, Req::Report). Nothing is sent unless the tester ticked it.
//! Secrets (access tokens, the Windows user name in paths) are blanked before anything leaves the PC.

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const SALT: &str = "c42a87b7ff07fb16aaf42b9495dc2a85";
const ROUNDS: u32 = 200_000;
/// sha256 of the unlock tag; the tag itself (kept once unlocked) can only be made from the password.
const CHECK: &str = "f2c51b77ef30a9d27b8f085e7f933906ed49b918e8411eab5b22f269c730b949";
/// The biggest report text accepted (a long game log); the start and end of anything longer are kept.
pub const MAX_TEXT: usize = 8 << 20;

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 { k[..32].copy_from_slice(&Sha256::digest(key)) } else { k[..key.len()].copy_from_slice(key) }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    for p in parts { inner.update(p) }
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// PBKDF2-HMAC-SHA256, one 32-byte block.
fn pbkdf2(pw: &[u8], salt: &[u8], rounds: u32) -> [u8; 32] {
    let mut u = hmac(pw, &[salt, &1u32.to_be_bytes()]);
    let mut out = u;
    for _ in 1..rounds {
        u = hmac(pw, &[&u]);
        for (o, x) in out.iter_mut().zip(u) { *o ^= x }
    }
    out
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).filter_map(|i| u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()).collect()
}

fn tag_ok(tag: &str) -> bool {
    tag.len() == 64 && hex(&Sha256::digest(unhex(tag))) == CHECK
}

/// The unlock tag for this password, or None when it's wrong.
pub fn unlock(pw: &str) -> Option<String> {
    let dk = pbkdf2(pw.as_bytes(), &unhex(SALT), ROUNDS);
    let tag = hex(&Sha256::digest([b"obelisk-dev-tag".as_slice(), &dk].concat()));
    tag_ok(&tag).then_some(tag)
}

/// What the tester ticked, plus how far each kind of report has been sent.
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct DevState {
    tag: Option<String>,
    pub crash: bool,
    pub logs: bool,
    pub mods: bool,
    pub client: bool,
    /// "<kind>:<pack id>" -> the newest file time (ms) or content hash already sent.
    pub marks: HashMap<String, String>,
    /// The last test or send, for the tab to show.
    pub last: Option<String>,
}

impl DevState {
    pub fn unlocked(&self) -> bool {
        self.tag.as_deref().is_some_and(tag_ok)
    }
    pub fn set_tag(&mut self, tag: String) {
        self.tag = Some(tag);
    }
    pub fn lock(&mut self) {
        *self = DevState::default();
    }
    pub fn any(&self) -> bool {
        self.unlocked() && (self.crash || self.logs || self.mods || self.client)
    }
}

pub fn load(file: &Path) -> DevState {
    std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(file: &Path, s: &DevState) -> Result<(), String> {
    if let Some(d) = file.parent() { std::fs::create_dir_all(d).map_err(|e| e.to_string())? }
    std::fs::write(file, serde_json::to_string_pretty(s).unwrap_or_default()).map_err(|e| e.to_string())
}

/// Blank what must never leave the PC: access/session tokens, sign-in JWTs, the Windows user name in paths.
pub fn scrub(text: &str, user: Option<&str>) -> String {
    let mut t = text.to_string();
    if let Some(u) = user.filter(|u| u.len() >= 2) {
        for sep in ["\\", "/"] {
            for (from, to) in [(format!("Users{sep}{u}"), format!("Users{sep}<user>")), (format!("users{sep}{u}"), format!("users{sep}<user>"))] {
                t = t.replace(&from, &to);
            }
        }
    }
    for marker in ["--accessToken ", "accessToken=", "accessToken\":\"", "\"accessToken\": \"", "access_token=", "access_token\":\"",
        "\"access_token\": \"", "token:", "Bearer ", "--uuid ", "--xuid ", "--clientId ", "refresh_token\":\"", "\"refresh_token\": \"", "--session "] {
        t = hide_after(&t, marker);
    }
    hide_jwts(&t)
}

fn secret_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-_.+/=".contains(c)
}

fn hide_after(text: &str, marker: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(marker) {
        let after = &rest[i + marker.len()..];
        let n = after.find(|c: char| !secret_char(c)).unwrap_or(after.len());
        out.push_str(&rest[..i + marker.len()]);
        if n >= 6 { out.push_str("[hidden]") } else { out.push_str(&after[..n]) }
        rest = &after[n..];
    }
    out.push_str(rest);
    out
}

fn hide_jwts(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("eyJ") {
        let after = &rest[i..];
        let n = after.find(|c: char| !(c.is_ascii_alphanumeric() || "-_.".contains(c))).unwrap_or(after.len());
        out.push_str(&rest[..i]);
        if n >= 30 { out.push_str("[hidden]") } else { out.push_str(&after[..n]) }
        rest = &after[n..];
    }
    out.push_str(rest);
    out
}

/// One report on its way: `gz` is the scrubbed text, gzipped and base64'd.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub kind: String,
    pub name: String,
    pub pack: String,
    pub version: String,
    pub gz: String,
}

impl Report {
    pub fn new(kind: &str, name: &str, pack: &str, version: &str, text: &str) -> Report {
        let text = clip(text);
        let mut z = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let _ = z.write_all(text.as_bytes());
        let gz = base64::engine::general_purpose::STANDARD.encode(z.finish().unwrap_or_default());
        Report { kind: kind.into(), name: name.into(), pack: pack.into(), version: version.into(), gz }
    }

    pub fn text(&self) -> Result<String, String> {
        let raw = base64::engine::general_purpose::STANDARD.decode(&self.gz).map_err(|_| "bad report")?;
        let mut s = String::new();
        flate2::read::GzDecoder::new(raw.as_slice()).take(MAX_TEXT as u64 + 1).read_to_string(&mut s).map_err(|_| "bad report")?;
        if s.len() > MAX_TEXT { return Err("report too big".into()) }
        Ok(s)
    }
}

fn clip(text: &str) -> String {
    if text.len() <= MAX_TEXT { return text.to_string() }
    let keep = MAX_TEXT / 2 - 64;
    let mut a = keep;
    while !text.is_char_boundary(a) { a -= 1 }
    let mut b = text.len() - keep;
    while !text.is_char_boundary(b) { b += 1 }
    format!("{}\n\n[... {} bytes cut by Obelisk ...]\n\n{}", &text[..a], b - a, &text[b..])
}

fn safe(s: &str) -> String {
    let s: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).take(80).collect();
    if s.is_empty() || s.chars().all(|c| c == '.') { "_".into() } else { s }
}

/// On the owner's PC: keep a member's report under `dir/<name>-<id>/`, with a line in index.jsonl. Returns the file.
pub fn store(dir: &Path, from_name: &str, from_id: &str, r: &Report, now_ms: u64) -> Result<PathBuf, String> {
    let text = r.text()?;
    let who = dir.join(format!("{}-{}", safe(from_name), safe(&from_id.chars().take(8).collect::<String>())));
    std::fs::create_dir_all(&who).map_err(|e| e.to_string())?;
    let file = who.join(format!("{now_ms}-{}-{}", safe(&r.kind), safe(&r.name)));
    std::fs::write(&file, &text).map_err(|e| e.to_string())?;
    let line = serde_json::json!({ "time": now_ms, "kind": r.kind, "name": r.name, "pack": r.pack, "version": r.version,
        "bytes": text.len(), "file": file.file_name().map(|n| n.to_string_lossy().to_string()) });
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(who.join("index.jsonl")) {
        let _ = writeln!(f, "{line}");
    }
    prune(&who, 3000);
    Ok(file)
}

/// Keep only the newest `keep` report files of one sender.
fn prune(who: &Path, keep: usize) {
    let mut files: Vec<PathBuf> = std::fs::read_dir(who).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n != "index.jsonl")).collect();
    if files.len() <= keep { return }
    files.sort();
    for f in &files[..files.len() - keep] { let _ = std::fs::remove_file(f); }
}

fn mtime_ms(p: &Path) -> u64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Something new to send, and the mark to keep once it arrived.
#[derive(Debug, Clone)]
pub struct Item {
    pub kind: &'static str,
    pub name: String,
    pub text: String,
    pub mark: (String, String),
}

fn mark_num(s: &DevState, key: &str) -> u64 {
    s.marks.get(key).and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// What's new in one shared pack's game folder since the marks, for the kinds the tester ticked. Logs only while
/// the game is closed (a running game is still writing them). A first send marks what's there as seen first,
/// so ticking a box doesn't upload years of old logs.
pub fn collect(game: &Path, pack: &str, s: &DevState, running: bool) -> Vec<Item> {
    let mut out = vec![];
    let newer = |key: &str, files: Vec<PathBuf>, kind: &'static str, out: &mut Vec<Item>| {
        let since = s.marks.get(key).and_then(|v| v.parse::<u64>().ok());
        let newest = files.iter().map(|f| mtime_ms(f)).max().unwrap_or(0);
        let Some(since) = since else {
            out.push(Item { kind: "", name: String::new(), text: String::new(), mark: (key.to_string(), newest.to_string()) });
            return;
        };
        for f in files.iter().filter(|f| mtime_ms(f) > since) {
            let Ok(bytes) = std::fs::read(f) else { continue };
            out.push(Item { kind, name: f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                text: String::from_utf8_lossy(&bytes).to_string(), mark: (key.to_string(), mtime_ms(f).max(since).to_string()) });
        }
    };
    let list = |dir: PathBuf, ok: &dyn Fn(&str) -> bool| -> Vec<PathBuf> {
        std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path())
            .filter(|p| p.is_file() && p.file_name().is_some_and(|n| ok(&n.to_string_lossy()))).collect()
    };
    if s.crash {
        let mut f = list(game.join("crash-reports"), &|n| n.ends_with(".txt"));
        f.extend(list(game.to_path_buf(), &|n| n.starts_with("hs_err_pid") && n.ends_with(".log")));
        newer(&format!("crash:{pack}"), f, "crash", &mut out);
    }
    if s.logs && !running {
        newer(&format!("logs:{pack}"), list(game.join("logs"), &|n| n == "latest.log" || n == "debug.log"), "log", &mut out);
    }
    if s.mods {
        let mut mods: Vec<String> = list(game.join("mods"), &|n| n.ends_with(".jar") || n.ends_with(".disabled"))
            .iter().map(|p| format!("{}\t{}", p.file_name().unwrap_or_default().to_string_lossy(), std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)))
            .collect();
        mods.sort();
        let text = mods.join("\n");
        let key = format!("mods:{pack}");
        let h = hex(&Sha256::digest(text.as_bytes()))[..16].to_string();
        if s.marks.get(&key) != Some(&h) {
            out.push(Item { kind: "mods", name: "mods.txt".into(), text, mark: (key, h) });
        }
    }
    out
}

/// Obelisk's own errors (errors.jsonl lines) newer than the mark, as one item. First time: only marks.
pub fn collect_client(errors_file: &Path, s: &DevState) -> Option<Item> {
    if !s.client { return None }
    let key = "client".to_string();
    let text = std::fs::read_to_string(errors_file).unwrap_or_default();
    let time = |l: &str| serde_json::from_str::<serde_json::Value>(l).ok().and_then(|v| v["time"].as_u64()).unwrap_or(0);
    let newest = text.lines().map(time).max().unwrap_or(0);
    if !s.marks.contains_key(&key) {
        return Some(Item { kind: "", name: String::new(), text: String::new(), mark: (key, newest.to_string()) });
    }
    let since = mark_num(s, &key);
    let new: Vec<&str> = text.lines().filter(|l| time(l) > since).collect();
    (!new.is_empty()).then(|| Item { kind: "client", name: "obelisk-errors.jsonl".into(), text: new.join("\n"), mark: (key, newest.to_string()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbkdf2_matches_the_standard() {
        // RFC 6070-style vector for SHA-256 (password/salt, 4096 rounds).
        assert_eq!(hex(&pbkdf2(b"password", b"salt", 4096)), "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a");
    }

    #[test]
    fn wrong_passwords_and_forged_tags_are_refused() {
        for pw in ["", " ", "password", "admin", "letmein", "0000"] {
            assert!(unlock(pw).is_none(), "{pw:?} unlocked");
        }
        let mut s = DevState { crash: true, ..Default::default() };
        s.set_tag(CHECK.into());
        assert!(!s.unlocked(), "the stored check value is not a tag");
        s.set_tag("0".repeat(64));
        assert!(!s.unlocked());
        assert!(!s.any(), "nothing sends while locked, even with boxes ticked");
    }

    #[test]
    fn right_password_from_env_unlocks() {
        // Run by hand: OBELISK_DEV_PW=... cargo test --release devreport (the password lives in no file).
        let Ok(pw) = std::env::var("OBELISK_DEV_PW") else { return };
        let tag = unlock(&pw).expect("right password refused");
        let mut s = DevState::default();
        s.set_tag(tag);
        assert!(s.unlocked());
    }

    #[test]
    fn scrub_hides_tokens_and_user_name() {
        let jwt = format!("eyJhbGciOiJIUzI1NiJ9.{}.sig_part_here", "a".repeat(40));
        let t = format!("java --accessToken abcdef123456789 --uuid 1234567890abcdef --username Steve\n\
            path C:\\Users\\Example\\AppData\\x and C:/Users/Example/y\nSession ID is token:0123456789abcdef:fffff\nAuthorization: Bearer {jwt}\n\
            {{\"accessToken\":\"zzzzzzzzzzzz\",\"name\":\"ok\"}}\nplain {jwt} end\nshort token: ab");
        let s = scrub(&t, Some("Example"));
        for bad in ["abcdef123456789", "1234567890abcdef", "Example", "0123456789abcdef", "zzzzzzzzzzzz", "eyJhbGci"] {
            assert!(!s.contains(bad), "{bad} left in:\n{s}");
        }
        assert!(s.contains("--username Steve") && s.contains("\"name\":\"ok\"") && s.contains("short token: ab"), "{s}");
    }

    #[test]
    fn report_round_trips_and_store_is_contained() {
        let d = tempfile::tempdir().unwrap();
        let r = Report::new("crash", "../../evil.txt", "pack", "0.2.173", "boom");
        assert_eq!(r.text().unwrap(), "boom");
        let f = store(d.path(), "../Buddy", "abcdef0123456789", &r, 5).unwrap();
        assert!(f.starts_with(d.path()), "{}", f.display());
        assert_eq!(f.parent().unwrap().parent().unwrap(), d.path());
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "boom");
        assert!(f.parent().unwrap().join("index.jsonl").is_file());
        let bad = Report { gz: "!!!".into(), ..r.clone() };
        assert!(store(d.path(), "b", "c", &bad, 6).is_err());
        let big = Report::new("log", "latest.log", "p", "v", &"x".repeat(MAX_TEXT + 1000));
        assert!(big.text().unwrap().len() <= MAX_TEXT && big.text().unwrap().contains("cut by Obelisk"));
    }

    #[test]
    fn collect_sends_only_ticked_new_things() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        std::fs::create_dir_all(g.join("crash-reports")).unwrap();
        std::fs::create_dir_all(g.join("logs")).unwrap();
        std::fs::create_dir_all(g.join("mods")).unwrap();
        std::fs::write(g.join("crash-reports/old.txt"), "old").unwrap();
        std::fs::write(g.join("logs/latest.log"), "log").unwrap();
        std::fs::write(g.join("mods/a.jar"), "a").unwrap();
        let mut s = DevState::default();
        assert!(collect(g, "p", &s, false).is_empty(), "nothing ticked = nothing");
        s.crash = true; s.logs = true; s.mods = true;
        let first = collect(g, "p", &s, false);
        assert!(first.iter().all(|i| i.kind.is_empty() || i.kind == "mods"), "old crashes/logs only marked: {first:?}");
        for i in &first { s.marks.insert(i.mark.0.clone(), i.mark.1.clone()); }
        assert!(collect(g, "p", &s, false).is_empty(), "nothing new");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(g.join("crash-reports/new.txt"), "new crash").unwrap();
        std::fs::write(g.join("logs/latest.log"), "new log").unwrap();
        let running = collect(g, "p", &s, true);
        assert_eq!(running.iter().map(|i| i.kind).collect::<Vec<_>>(), vec!["crash"], "no logs while running");
        let closed = collect(g, "p", &s, false);
        assert_eq!(closed.len(), 2);
        assert!(closed.iter().any(|i| i.name == "new.txt" && i.text == "new crash"));
        assert!(closed.iter().any(|i| i.name == "latest.log" && i.text == "new log"));
    }

    #[test]
    fn client_errors_only_new_lines() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("errors.jsonl");
        std::fs::write(&f, "{\"time\":1,\"message\":\"a\"}\n").unwrap();
        let mut s = DevState { client: true, ..Default::default() };
        let i = collect_client(&f, &s).unwrap();
        assert!(i.kind.is_empty());
        s.marks.insert(i.mark.0, i.mark.1);
        assert!(collect_client(&f, &s).is_none());
        std::fs::write(&f, "{\"time\":1,\"message\":\"a\"}\n{\"time\":9,\"message\":\"b\"}\n").unwrap();
        let i = collect_client(&f, &s).unwrap();
        assert_eq!(i.text, "{\"time\":9,\"message\":\"b\"}");
    }
}
