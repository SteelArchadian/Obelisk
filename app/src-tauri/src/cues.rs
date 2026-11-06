//! Social sound cues (obelisk-social/OUTLINE.md, "Sound cues"): one small family of glassy chimes, all in F# major
//! pentatonic so they sound like a set, apart from the update jingles. The social framework calls `cue(id)` for each
//! event; settings decide whether and how loud it plays. Settings key `cues`:
//! `{ on: bool, volume: 0-100, dnd: bool, muted: [ids] }` (defaults on, 70, off, none).

use crate::sound::{n, render, Voice};
use std::time::{Duration, Instant};

/// Every cue, in the order Settings lists them: (id, label, synth).
pub const CUES: [(&str, &str, fn() -> Vec<f32>); 13] = [
    ("message", "New message", message), ("message_sent", "Message sent", message_sent), ("mention", "Mention", mention),
    ("friend_request", "Friend request", friend_request), ("friend_accepted", "Friend request accepted", friend_accepted),
    ("friend_online", "Friend came online", friend_online), ("invite", "Game invite", invite),
    ("invite_accepted", "Invite accepted", invite_accepted), ("invite_declined", "Invite declined", invite_declined),
    ("joined_world", "Joined a friend's world", joined_world), ("disconnected", "Disconnected", disconnected),
    ("reconnected", "Reconnected", reconnected), ("error", "Something went wrong", error),
];

/// Bursts: within this long of the last sound of a group, another plays nothing.
const COALESCE: Duration = Duration::from_secs(3);

/// Plays cue `id` (unless settings say not to) on its own thread; safe to call from any event handler.
pub fn cue(app: &tauri::AppHandle, id: &str) {
    let s = crate::settings_value(app);
    let c = s.get("cues");
    let get = |k: &str| c.and_then(|c| c.get(k));
    let on = get("on").and_then(|v| v.as_bool()).unwrap_or(true);
    let dnd = get("dnd").and_then(|v| v.as_bool()).unwrap_or(false);
    let muted = get("muted").and_then(|v| v.as_array()).is_some_and(|m| m.iter().any(|v| v.as_str() == Some(id)));
    let invite = id == "invite";
    // Do Not Disturb silences everything but invites; the master switch and a muted cue silence even those.
    if !on || muted || (dnd && !invite) { return }
    let Some(&(_, _, synth)) = CUES.iter().find(|c| c.0 == id) else { return };
    let mut volume = get("volume").and_then(|v| v.as_f64()).unwrap_or(70.0).clamp(0.0, 100.0) / 100.0;
    if !invite && game_in_front() {
        // While playing: the small stuff is skipped, the rest is played at a third of its level.
        if matches!(id, "friend_online" | "message_sent" | "reconnected") { return }
        volume /= 3.0;
    }
    if !fresh(id) { return }
    std::thread::spawn(move || {
        let pcm: Vec<f32> = synth().into_iter().map(|v| v * volume as f32).collect();
        crate::sound::play(&pcm);
    });
}

/// Settings' preview buttons: one cue now, at the set volume, ignoring Do Not Disturb, mutes and grouping.
#[tauri::command]
pub fn cue_preview(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let synth = CUES.iter().find(|c| c.0 == id).ok_or_else(|| format!("No cue called \"{id}\""))?.2;
    let volume = crate::settings_value(&app).pointer("/cues/volume").and_then(|v| v.as_f64()).unwrap_or(70.0) / 100.0;
    std::thread::spawn(move || crate::sound::play(&synth().into_iter().map(|v| v * volume as f32).collect::<Vec<_>>()));
    Ok(())
}

/// False when a cue of the same group sounded in the last 3 s: messages and mentions count as one group (a mention
/// still plays over plain messages), the rest each their own.
fn fresh(id: &str) -> bool {
    static LAST: std::sync::Mutex<Vec<(&'static str, Instant)>> = std::sync::Mutex::new(Vec::new());
    let group: &'static str = match id { "message" | "mention" => "message", _ => CUES.iter().find(|c| c.0 == id).map_or("", |c| c.0) };
    let mut last = LAST.lock().unwrap();
    let now = Instant::now();
    if let Some(i) = last.iter().position(|l| l.0 == group) {
        let quiet = now - last[i].1 < COALESCE && !(id == "mention" && last[i].0 == "message");
        last[i].1 = now; // a steady stream stays one sound, not one every 3 s
        if quiet { return false }
    } else {
        last.push((group, now));
    }
    true
}

/// Whether the window in front belongs to Java (Minecraft or another game), so cues stay out of the way.
#[cfg(windows)]
fn game_in_front() -> bool {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetForegroundWindow() -> isize;
        fn GetWindowThreadProcessId(h: isize, pid: *mut u32) -> u32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn QueryFullProcessImageNameW(p: isize, flags: u32, name: *mut u16, len: *mut u32) -> i32;
        fn CloseHandle(h: isize) -> i32;
    }
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), &mut pid);
        let p = OpenProcess(0x1000 /* PROCESS_QUERY_LIMITED_INFORMATION */, 0, pid);
        if p == 0 { return false }
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, 0, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(p);
        let exe = String::from_utf16_lossy(&buf[..len as usize]).to_ascii_lowercase();
        ok && exe.rsplit(['\\', '/']).next().is_some_and(|f| f.starts_with("java"))
    }
}

#[cfg(not(windows))]
fn game_in_front() -> bool { false }

// ---- the cues: glass chimes on F# major pentatonic (F# G# A# C# D#), each shape telling its event ----

const G: Voice = Voice::Glass;
/// The family's level: quiet like the jingles, with invites up and a friend coming online barely there.
const LEVEL: f32 = 0.08;

/// Two quick notes up a fourth: "hey".
fn message() -> Vec<f32> { render(&[n(0.0, 85.0, 0.12, 0.8, G), n(0.07, 90.0, 0.25, 1.0, G)], 5000.0, LEVEL) }

/// One tiny soft tick: sent.
fn message_sent() -> Vec<f32> { render(&[n(0.0, 92.0, 0.14, 1.0, G)], 5000.0, LEVEL * 0.6) }

/// The message figure with a third note on top: someone said your name.
fn mention() -> Vec<f32> {
    render(&[n(0.0, 85.0, 0.12, 0.8, G), n(0.06, 90.0, 0.12, 0.9, G), n(0.12, 94.0, 0.28, 1.0, G)], 5000.0, LEVEL * 1.15)
}

/// A low note answered twice from above: knock-knock.
fn friend_request() -> Vec<f32> {
    render(&[n(0.0, 78.0, 0.15, 0.8, G), n(0.1, 87.0, 0.15, 0.9, G), n(0.24, 78.0, 0.15, 0.7, G), n(0.34, 87.0, 0.3, 1.0, G)], 5000.0, LEVEL)
}

/// Up the chord to a ringing octave: friends now.
fn friend_accepted() -> Vec<f32> {
    let mut v = vec![n(0.0, 78.0, 0.2, 0.8, G), n(0.08, 82.0, 0.2, 0.85, G), n(0.16, 85.0, 0.2, 0.9, G)];
    v.extend([n(0.26, 90.0, 0.4, 1.0, G), n(0.26, 78.0, 0.4, 0.5, G)]);
    render(&v, 5000.0, LEVEL)
}

/// A single faint high glint.
fn friend_online() -> Vec<f32> { render(&[n(0.0, 92.0, 0.25, 1.0, G)], 4000.0, LEVEL * 0.45) }

/// The one meant to be heard: a bright call that climbs, answers itself and rings.
fn invite() -> Vec<f32> {
    let v = [
        n(0.0, 78.0, 0.18, 0.8, G), n(0.09, 85.0, 0.18, 0.9, G), n(0.18, 90.0, 0.18, 1.0, G), n(0.27, 85.0, 0.18, 0.8, G),
        n(0.4, 94.0, 0.4, 1.0, G), n(0.4, 90.0, 0.4, 0.6, G), n(0.4, 78.0, 0.4, 0.5, G),
    ];
    render(&v, 6000.0, LEVEL * 1.6)
}

/// A pickup into a bright two-note chord: they're coming.
fn invite_accepted() -> Vec<f32> {
    render(&[n(0.0, 78.0, 0.15, 0.8, G), n(0.1, 85.0, 0.4, 1.0, G), n(0.1, 90.0, 0.4, 0.8, G)], 5000.0, LEVEL)
}

/// Two notes stepping gently down: not this time.
fn invite_declined() -> Vec<f32> { render(&[n(0.0, 85.0, 0.15, 0.9, G), n(0.12, 82.0, 0.25, 0.8, G)], 4000.0, LEVEL * 0.8) }

/// A sweep up from low to high: you're in.
fn joined_world() -> Vec<f32> {
    let v: Vec<_> = [66.0, 73.0, 78.0, 82.0, 85.0].iter().enumerate()
        .map(|(i, &k)| n(i as f64 * 0.06, k, if i == 4 { 0.35 } else { 0.18 }, 0.7 + 0.06 * i as f64, G)).collect();
    render(&v, 5000.0, LEVEL)
}

/// Down a fourth, soft: the line dropped.
fn disconnected() -> Vec<f32> { render(&[n(0.0, 78.0, 0.15, 0.9, G), n(0.12, 73.0, 0.3, 0.8, G)], 3500.0, LEVEL * 0.85) }

/// The same fourth back up: connected again.
fn reconnected() -> Vec<f32> { render(&[n(0.0, 73.0, 0.15, 0.8, G), n(0.11, 78.0, 0.25, 0.9, G)], 3500.0, LEVEL * 0.8) }

/// The family's one sour note: two low notes a half step apart, gently. Not alarming, just "hm".
fn error() -> Vec<f32> { render(&[n(0.0, 74.0, 0.14, 0.9, G), n(0.13, 73.0, 0.28, 0.9, G)], 3000.0, LEVEL * 0.9) }
