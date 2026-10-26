//! Load-time timeline of each game session: what the launcher sees (click, Java start, window, exit) joined with the
//! milestones in the game's log and, when Speed Loader is installed, its own in-game marks. One JSON line per session
//! in `<gameDir>/obelisk-timeline.jsonl`, so Speed Loader and the chats can read it too.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::Path;

pub const FILE: &str = "obelisk-timeline.jsonl";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Mark {
    pub name: String,
    /// Unix time in milliseconds.
    pub ms: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Session {
    /// "obelisk" (Play button) or "test" (a chat's test run).
    pub source: String,
    pub label: String,
    pub click_ms: i64,
    pub java_ms: Option<i64>,
    pub window_ms: Option<i64>,
    pub exit_ms: Option<i64>,
    pub marks: Vec<Mark>,
    /// Speed Loader's bench.jsonl line for this session, when it wrote one.
    pub speedloader: Option<Value>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Phase {
    pub key: String,
    pub label: String,
    pub ms: i64,
    /// Time the player chose to spend (in the menu, playing), not loading.
    pub idle: bool,
}

/// What a log line means, by the text it contains. `world-*` repeat for every world joined in one session.
const LOG_MARKS: &[(&str, &str)] = &[
    ("Setting user:", "gameStart"),
    ("Sound engine started", "resourcesLoaded"),
    ("Starting integrated minecraft server", "worldStart"),
    ("Connecting to ", "worldStart"),
    (" joined the game", "inWorld"),
    ("Stopping server", "worldLeave"),
    ("All dimensions are saved", "worldSaved"),
    ("Stopping!", "quit"),
];

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468
}

/// "[15Oct2026 14:08:18.101] ..." -> that local time as if it were UTC, in ms.
fn line_time(line: &str) -> Option<i64> {
    let s = line.strip_prefix('[')?.get(..24)?;
    let (date, time) = s.split_once(' ')?;
    let d: i64 = date.get(..2)?.parse().ok()?;
    let mon = date.get(2..5)?;
    let m = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].iter().position(|x| *x == mon)? as i64 + 1;
    let y: i64 = date.get(5..9)?.parse().ok()?;
    let mut t = time.split([':', '.']);
    let (h, mi, se, ms): (i64, i64, i64, i64) = (t.next()?.parse().ok()?, t.next()?.parse().ok()?, t.next()?.parse().ok()?, t.next()?.get(..3)?.parse().ok()?);
    Some(((days_from_civil(y, m, d) * 24 + h) * 60 + mi) * 60_000 + se * 1000 + ms)
}

/// Local time minus UTC right now, in ms.
#[cfg(windows)]
pub fn local_offset_ms() -> i64 {
    #[repr(C)]
    #[derive(Default)]
    struct St { y: u16, mo: u16, dow: u16, d: u16, h: u16, mi: u16, s: u16, ms: u16 }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemTime(t: *mut St);
        fn SystemTimeToTzSpecificLocalTime(tz: *const u8, utc: *const St, local: *mut St) -> i32;
    }
    let (mut utc, mut local) = (St::default(), St::default());
    let ok = unsafe { GetSystemTime(&mut utc); SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) };
    if ok == 0 { return 0; }
    let ms = |t: &St| ((days_from_civil(t.y as i64, t.mo as i64, t.d as i64) * 24 + t.h as i64) * 60 + t.mi as i64) * 60_000 + t.s as i64 * 1000;
    ms(&local) - ms(&utc)
}

#[cfg(not(windows))]
pub fn local_offset_ms() -> i64 { 0 }

/// The milestones in a log's text, plus `logStart` for its first line. Times are Unix ms.
pub fn parse_log(text: &str, offset_ms: i64) -> Vec<Mark> {
    let mut out = vec![];
    for line in text.lines() {
        let Some(t) = line_time(line) else { continue };
        let ms = t - offset_ms;
        if out.is_empty() { out.push(Mark { name: "logStart".into(), ms }); }
        // Only the logger's message part, so a mod echoing chat or paths can't fake a mark.
        let msg = line.splitn(4, "]").nth(3).unwrap_or("");
        if let Some((_, name)) = LOG_MARKS.iter().find(|(k, _)| msg.contains(k)) {
            let once = matches!(*name, "gameStart" | "resourcesLoaded" | "quit");
            if !(once && out.iter().any(|m| m.name == *name)) {
                out.push(Mark { name: name.to_string(), ms });
            }
        }
    }
    out
}

/// Unix ms when a process started (exact, unlike sysinfo's whole seconds).
#[cfg(windows)]
pub fn process_start_ms(pid: u32) -> Option<i64> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn GetProcessTimes(h: isize, create: *mut u64, exit: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
        fn CloseHandle(h: isize) -> i32;
    }
    let (mut c, mut e, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
    unsafe {
        let h = OpenProcess(0x1000 /* QUERY_LIMITED_INFORMATION */, 0, pid);
        if h == 0 { return None; }
        let ok = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u);
        CloseHandle(h);
        if ok == 0 { return None; }
    }
    Some((c / 10_000) as i64 - 11_644_473_600_000)
}

#[cfg(not(windows))]
pub fn process_start_ms(_pid: u32) -> Option<i64> { None }

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// After a session ends: read its log (and Speed Loader's line), save the session, and return it.
pub fn record(game_dir: &Path, mut s: Session) -> Session {
    let text = fs::read(game_dir.join("logs").join("latest.log")).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let marks = parse_log(&text, local_offset_ms());
    // Only a log that this session wrote (a game that never got going leaves the previous one in place).
    if marks.first().is_some_and(|m| m.ms >= s.click_ms - 5_000) {
        s.marks = marks;
    }
    let end = s.exit_ms.unwrap_or_else(now_ms);
    let sl = fs::read_to_string(optimal_file(game_dir, "bench.jsonl")).unwrap_or_default();
    s.speedloader = sl.lines().rev().take(20).filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|v| v.get("jvmStart").and_then(Value::as_i64).is_some_and(|t| t >= s.click_ms - 5_000 && t <= end));
    s.marks.extend(speedloader_events(game_dir, s.click_ms - 5_000, end));
    if let Ok(line) = serde_json::to_string(&s) {
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(game_dir.join(FILE)) {
            let _ = writeln!(f, "{line}");
        }
    }
    s
}

/// Events so far in a game that started at `from` (Unix ms), oldest first: the log's marks plus Optimal's
/// exact ones ("sl.<event>"). For watching a run live; a log from an earlier session is ignored.
pub fn live_marks(game_dir: &Path, from: i64) -> Vec<Mark> {
    let text = fs::read(game_dir.join("logs").join("latest.log")).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let mut marks = parse_log(&text, local_offset_ms());
    if !marks.first().is_some_and(|m| m.ms >= from - 5_000) {
        marks.clear();
    }
    marks.extend(speedloader_events(game_dir, from - 5_000, now_ms()));
    marks.sort_by_key(|m| m.ms);
    marks
}

/// Optimal (formerly Speed Loader) moves speedloader/ to optimal/ on first start; older copies keep speedloader/.
fn optimal_file(game_dir: &Path, name: &str) -> std::path::PathBuf {
    let new = game_dir.join("optimal").join(name);
    if new.exists() { new } else { game_dir.join("speedloader").join(name) }
}

/// Speed Loader's per-session events (speedloader/timeline.jsonl, 0.1.6+) for the game that started in
/// [from, to], as marks named "sl.<event>" at Unix ms. Exact, so they beat the log-line guesses.
fn speedloader_events(game_dir: &Path, from: i64, to: i64) -> Vec<Mark> {
    let text = fs::read_to_string(optimal_file(game_dir, "timeline.jsonl")).unwrap_or_default();
    let lines: Vec<Value> = text.lines().rev().take(500).filter_map(|l| serde_json::from_str(l).ok()).collect();
    let Some(session) = lines.iter().filter_map(|v| v.get("session")?.as_i64()).find(|t| *t >= from && *t <= to) else { return vec![] };
    let mut out: Vec<Mark> = lines.iter().rev()
        .filter(|v| v.get("session").and_then(Value::as_i64) == Some(session))
        .filter_map(|v| Some(Mark { name: format!("sl.{}", v.get("event")?.as_str()?), ms: session + v.get("t")?.as_i64()? }))
        .collect();
    out.sort_by_key(|m| m.ms);
    out
}

/// The newest `n` sessions, newest first.
pub fn sessions(game_dir: &Path, n: usize) -> Vec<Session> {
    let text = fs::read_to_string(game_dir.join(FILE)).unwrap_or_default();
    text.lines().rev().filter_map(|l| serde_json::from_str(l).ok()).take(n).collect()
}

/// A session as phases, in order: launcher, Java, mods, resources, then menu/join/play/leave per world, then quit.
#[allow(unused_assignments)] // the last join leaves `start` cleared
pub fn phases(s: &Session) -> Vec<Phase> {
    let mut out = vec![];
    let mut push = |key: &str, label: &str, from: Option<i64>, to: Option<i64>, idle: bool| {
        if let (Some(a), Some(b)) = (from, to) {
            if b >= a { out.push(Phase { key: key.into(), label: label.into(), ms: b - a, idle }); }
        }
    };
    let first = |name: &str| s.marks.iter().find(|m| m.name == name).map(|m| m.ms);
    let sl = |k: &str| -> Option<i64> {
        let v = s.speedloader.as_ref()?;
        Some(v.get("jvmStart")?.as_i64()? + v.get(k)?.as_i64()?)
    };
    let java = s.java_ms.or_else(|| s.speedloader.as_ref()?.get("jvmStart")?.as_i64());
    push("launcher", "Launcher and Prism", Some(s.click_ms), java, false);
    push("jvm", "Java starting", java, first("logStart"), false);
    push("mods", "Loading mods", first("logStart"), first("gameStart"), false);
    // Speed Loader's title screen is exact; the sound engine starting is the closest log line to it.
    let ev = |name: &str| s.marks.iter().find(|m| m.name == name).map(|m| m.ms);
    let menu = ev("sl.titleScreen").or(sl("titleScreen")).or(first("resourcesLoaded"));
    push("resources", "Loading resources (to the menu)", first("gameStart"), menu, false);
    // World events: Speed Loader's own (every join, exact) when it wrote them, else the log lines.
    let exact = s.marks.iter().any(|m| m.name.starts_with("sl.") && m.name != "sl.titleScreen" && !m.name.starts_with("sl.pressure"));
    let quit = if exact { ev("sl.quitStart").or(first("quit")) } else { first("quit") };
    let events: Vec<(&str, i64)> = s.marks.iter().filter_map(|m| {
        let e = if exact {
            m.name.strip_prefix("sl.")?
        } else {
            match m.name.as_str() { "worldStart" => "joinStart", "inWorld" => "inWorld", "worldLeave" => "leaveStart", "worldSaved" => "leaveDone", _ => return None }
        };
        Some((e, m.ms))
    }).collect();
    let mut last = menu;
    let mut world = 0;
    let (mut start, mut entered) = (None, None);
    let mut in_world = false;
    macro_rules! joined { ($ready:expr) => {{
        let ready = $ready;
        push("join", &format!("Joining world {world}"), start, Some(ready), false);
        last = Some(ready);
        start = None;
    }}; }
    // Quitting from inside a world saves it after "Stopping!"/quitStart; that saving belongs to quitting.
    for (e, ms) in events.into_iter().filter(|(_, ms)| quit.is_none_or(|q| *ms < q)) {
        match e {
            "joinStart" => {
                world += 1;
                push("menu", "In the menu", last, Some(ms), true);
                (start, entered) = (Some(ms), None);
            }
            // The log only knows when you joined; Speed Loader also knows when the terrain around you is drawn.
            "inWorld" if start.is_some() => {
                entered = Some(ms);
                if !exact {
                    let ready = if world == 1 { sl("terrainReady").filter(|t| *t >= ms).unwrap_or(ms) } else { ms };
                    joined!(ready);
                    in_world = true;
                }
            }
            "terrainReady" if start.is_some() => { joined!(ms); in_world = true; }
            "leaveStart" => {
                if let Some(t) = entered.filter(|_| start.is_some()) { joined!(t); }
                push("play", &format!("Playing world {world}"), last, Some(ms), true);
                last = Some(ms);
                in_world = false;
            }
            "leaveDone" => { push("leave", &format!("Leaving world {world}"), last, Some(ms), false); last = Some(ms); }
            _ => {}
        }
    }
    if let Some(t) = entered.filter(|_| start.is_some()) {
        joined!(t);
        in_world = true;
    }
    if in_world {
        push("play", &format!("Playing world {world}"), last, quit, true);
    } else {
        push("menu", "In the menu", last, quit, true);
    }
    push("quit", "Quitting", quit, s.exit_ms, false);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "[15Oct2026 14:08:18.101] [main/INFO] [cpw.mods.modlauncher.Launcher/MODLAUNCHER]: ModLauncher running\n\
[15Oct2026 14:08:49.229] [Render thread/INFO] [net.minecraft.client.Minecraft/]: Setting user: PerfTest\n\
[15Oct2026 14:09:20.110] [Render thread/INFO] [net.minecraft.client.sounds.SoundEngine/SOUNDS]: Sound engine started\n\
[15Oct2026 14:09:35.964] [Server thread/INFO] [net.minecraft.client.server.IntegratedServer/]: Starting integrated minecraft server version 1.21.1\n\
[15Oct2026 14:09:46.367] [Server thread/INFO] [net.minecraft.server.MinecraftServer/]: PerfTest joined the game\n\
[15Oct2026 14:15:58.339] [Render thread/INFO] [net.minecraft.client.Minecraft/]: Stopping!\n\
[15Oct2026 14:15:58.813] [Server thread/INFO] [net.minecraft.server.MinecraftServer/]: Stopping server\n\
[15Oct2026 14:16:00.449] [Server thread/INFO] [net.minecraft.server.MinecraftServer/]: ThreadedAnvilChunkStorage: All dimensions are saved\n";

    #[test]
    fn log_marks_and_phases() {
        let marks = parse_log(LOG, 0);
        let names: Vec<_> = marks.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["logStart", "gameStart", "resourcesLoaded", "worldStart", "inWorld", "quit", "worldLeave", "worldSaved"]);
        let t0 = marks[0].ms;
        let s = Session { click_ms: t0 - 4_000, java_ms: Some(t0 - 1_500), exit_ms: Some(marks[5].ms + 3_000), marks, ..Default::default() };
        let p: Vec<(String, i64)> = phases(&s).into_iter().map(|p| (p.key, p.ms)).collect();
        assert_eq!(p[0], ("launcher".into(), 2_500));
        assert_eq!(p[1], ("jvm".into(), 1_500));
        assert_eq!(p[2], ("mods".into(), 31_128));
        assert_eq!(p[3], ("resources".into(), 30_881));
        assert_eq!(p[4], ("menu".into(), 15_854));
        assert_eq!(p[5], ("join".into(), 10_403));
        assert_eq!(p[6], ("play".into(), 371_972));
        assert_eq!(p.last().unwrap(), &("quit".into(), 3_000));
        assert_eq!(p.len(), 8);
    }

    #[test]
    fn speedloader_events_win() {
        let mut marks = parse_log(LOG, 0);
        let t0 = marks[0].ms;
        let jvm = t0 - 1_000;
        // Two joins, the second left without terrainReady (inWorld stands in), then quit from the menu.
        for (e, t) in [("titleScreen", 60_000), ("joinStart", 70_000), ("inWorld", 78_000), ("terrainReady", 80_000), ("leaveStart", 100_000),
                       ("leaveDone", 102_000), ("joinStart", 110_000), ("inWorld", 115_000), ("leaveStart", 130_000), ("leaveDone", 131_000), ("pressure2", 135_000), ("quitStart", 140_000)] {
            marks.push(Mark { name: format!("sl.{e}"), ms: jvm + t });
        }
        marks.sort_by_key(|m| m.ms);
        let s = Session { click_ms: jvm - 2_000, java_ms: Some(jvm), exit_ms: Some(jvm + 142_000), marks, ..Default::default() };
        let p: Vec<(String, i64)> = phases(&s).into_iter().map(|p| (p.key, p.ms)).collect();
        let keys: Vec<&str> = p.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["launcher", "jvm", "mods", "resources", "menu", "join", "play", "leave", "menu", "join", "play", "leave", "menu", "quit"]);
        assert_eq!(p[5].1, 10_000);
        assert_eq!(p[7].1, 2_000);
        assert_eq!(p[9].1, 5_000);
        assert_eq!(p[13].1, 2_000);
    }

    #[test]
    fn civil_days() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(line_time("[01Jan1970 00:00:01.250] x"), Some(1_250));
    }
}
