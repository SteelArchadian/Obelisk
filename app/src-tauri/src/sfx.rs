//! Sound themes (Settings → Sound → Sound theme): each one a family of short sounds on one instrument, sharing a name
//! with its own music track (Rainy Day plays with Puddle, Boutique with Boutique). A theme gives the update sound, the
//! menu sounds (hover, click, open, close, tab), the typing tick and the alert cues (cues.rs re-voices its cue shapes
//! on the theme's instrument). Settings can take everything from one theme, or pick each sound from any theme (user,
//! 2026-11-06: "the crystal new message and the breeze new mention"). Menu and typing sounds play in the UI
//! (uisound.svelte.ts), which fetches them from `ui_sfx` as WAV once and keeps them.

use crate::sound::{bend, n, render, ring_out, Note, Voice, RATE};

/// The themes, in the order Settings lists them, each with its sounds by kind.
pub const THEMES: [(&str, [(&str, fn() -> Vec<f32>); 7]); 6] = [
    ("rainy-day", [("update", rd_update), ("hover", rd_hover), ("click", rd_click), ("open", rd_open), ("close", rd_close), ("tab", rd_tab), ("type", rd_type)]),
    ("boutique", [("update", bq_update), ("hover", bq_hover), ("click", bq_click), ("open", bq_open), ("close", bq_close), ("tab", bq_tab), ("type", bq_type)]),
    ("lounge-keys", [("update", lk_update), ("hover", lk_hover), ("click", lk_click), ("open", lk_open), ("close", lk_close), ("tab", lk_tab), ("type", lk_type)]),
    ("island", [("update", island_update), ("hover", island_hover), ("click", island_click), ("open", island_open), ("close", island_close), ("tab", island_tab), ("type", island_type)]),
    ("crystal", [("update", crystal_update), ("hover", crystal_hover), ("click", crystal_click), ("open", crystal_open), ("close", crystal_close), ("tab", crystal_tab), ("type", crystal_type)]),
    ("bubblegum", [("update", gum_update), ("hover", gum_hover), ("click", gum_click), ("open", gum_open), ("close", gum_close), ("tab", gum_tab), ("type", gum_type)]),
];

/// One theme's sound of one kind, if both exist.
pub fn sound(theme: &str, kind: &str) -> Option<Vec<f32>> {
    let (_, kinds) = THEMES.iter().find(|t| t.0 == theme)?;
    Some(kinds.iter().find(|k| k.0 == kind)?.1())
}

/// A theme's menu or typing sound as a WAV file, for the UI to decode and keep.
#[tauri::command]
pub async fn ui_sfx(theme: String, kind: String) -> Result<tauri::ipc::Response, String> {
    // Synthesised off the window's thread, so the window never stalls while a theme's sounds load.
    tauri::async_runtime::spawn_blocking(move || {
        let pcm = sound(&theme, &kind).ok_or_else(|| format!("No {kind} sound in theme \"{theme}\""))?;
        Ok(tauri::ipc::Response::new(crate::sound::wav(&pcm)))
    }).await.map_err(|e| e.to_string())?
}

/// How a theme plays the alert cues: its instrument, how far to shift the cue shapes (semitones), a slide on each
/// note, and how much longer each note rings than the glass chime's.
pub struct Kit { pub voice: Voice, pub shift: f64, pub slide: f64, pub ring: f64 }

impl Kit {
    pub fn note(&self, at: f64, key: f64, len: f64, vol: f64) -> Note { bend(at, key + self.shift, len * self.ring, vol, self.voice, self.slide) }

    /// Renders a cue, its tail faded so a ringing instrument never stops short.
    pub fn render(&self, notes: &[Note], cutoff: f64, peak: f32) -> Vec<f32> {
        let pcm = render(notes, cutoff, peak);
        if self.ring > 1.0 { fade_tail(pcm, 0.15) } else { pcm }
    }
}

/// The default cues' glass chimes.
pub const CHIMES: Kit = Kit { voice: Voice::Glass, shift: 0.0, slide: 0.0, ring: 1.0 };

/// A theme's cue kit (None: not a theme).
pub fn kit(theme: &str) -> Option<Kit> {
    Some(match theme {
        "rainy-day" => Kit { voice: Voice::Keys, shift: -12.0, slide: 0.0, ring: 2.5 },
        "boutique" => Kit { voice: Voice::Harp, shift: -12.0, slide: 0.0, ring: 3.0 },
        "lounge-keys" => Kit { voice: Voice::Vibe, shift: -12.0, slide: 0.0, ring: 2.5 },
        "island" => Kit { voice: Voice::SteelPan, shift: -12.0, slide: 0.0, ring: 2.5 },
        "crystal" => Kit { voice: Voice::Celesta, shift: 0.0, slide: 0.0, ring: 2.5 },
        "bubblegum" => Kit { voice: Voice::Triangle, shift: -12.0, slide: 4.0, ring: 1.2 },
        _ => return None,
    })
}

/// Fades the last `secs` to silence (short sounds get a short fade, so they still start crisp).
fn fade_tail(mut pcm: Vec<f32>, secs: f64) -> Vec<f32> {
    let fade = ((secs * RATE as f64) as usize).min(pcm.len()).max(1);
    let len = pcm.len();
    for (i, v) in pcm.iter_mut().enumerate().skip(len - fade) {
        let x = (len - i) as f32 / fade as f32;
        *v *= x * x;
    }
    pcm
}

fn jingle(notes: &[Note], cutoff: f64, peak: f32) -> Vec<f32> { ring_out(render(notes, cutoff, peak)) }

fn arp(start: f64, step: f64, keys: &[f64], len: f64, vol: f64, voice: Voice) -> Vec<Note> {
    keys.iter().enumerate().map(|(i, &k)| n(start + i as f64 * step, k, len, vol, voice)).collect()
}

/// A short sound at a kind's level, its last `tail` seconds faded.
fn sfx(notes: &[Note], cutoff: f64, peak: f32, tail: f64) -> Vec<f32> { fade_tail(render(notes, cutoff, peak), tail) }

// Levels for each kind, so a theme's sounds sit together: hover and typing faintest, the update fullest.
const HOVER: f32 = 0.04;
const TYPE: f32 = 0.04;
const CLICK: f32 = 0.07;
const NAV: f32 = 0.08;

// Rainy Day: Puddle's electric piano and soft water drops.
const K: Voice = Voice::Keys;
const D: Voice = Voice::Triangle;
fn rd_update() -> Vec<f32> { jingle(&[n(0.0, 62.0, 1.2, 0.6, K), n(0.0, 66.0, 1.2, 0.5, K), n(0.0, 69.0, 1.2, 0.5, K), bend(0.18, 76.0, 0.22, 0.8, D, 7.0)], 2400.0, 0.11) }
fn rd_hover() -> Vec<f32> { sfx(&[bend(0.0, 79.0, 0.05, 1.0, D, 2.0)], 2600.0, HOVER, 0.02) }
fn rd_click() -> Vec<f32> { sfx(&[n(0.0, 74.0, 0.25, 1.0, K)], 2600.0, CLICK, 0.08) }
fn rd_open() -> Vec<f32> { sfx(&[bend(0.0, 72.0, 0.16, 1.0, D, 5.0), n(0.06, 69.0, 0.3, 0.6, K)], 2600.0, NAV, 0.1) }
fn rd_close() -> Vec<f32> { sfx(&[bend(0.0, 77.0, 0.16, 1.0, D, -5.0), n(0.06, 62.0, 0.3, 0.6, K)], 2600.0, NAV, 0.1) }
fn rd_tab() -> Vec<f32> { sfx(&[bend(0.0, 76.0, 0.1, 1.0, D, 3.0)], 2600.0, NAV, 0.04) }
fn rd_type() -> Vec<f32> { sfx(&[bend(0.0, 84.0, 0.035, 1.0, D, 1.0)], 2600.0, TYPE, 0.015) }

// Boutique: the harp, kept low and short so it stays soft.
const H: Voice = Voice::Harp;
fn bq_update() -> Vec<f32> { jingle(&arp(0.0, 0.07, &[67.0, 71.0, 74.0], 1.0, 0.7, H), 2400.0, 0.11) }
fn bq_hover() -> Vec<f32> { sfx(&[n(0.0, 79.0, 0.08, 1.0, H)], 2600.0, HOVER, 0.03) }
fn bq_click() -> Vec<f32> { sfx(&[n(0.0, 71.0, 0.2, 1.0, H)], 2600.0, CLICK, 0.08) }
fn bq_open() -> Vec<f32> { sfx(&arp(0.0, 0.05, &[67.0, 74.0], 0.3, 1.0, H), 2600.0, NAV, 0.12) }
fn bq_close() -> Vec<f32> { sfx(&arp(0.0, 0.05, &[74.0, 67.0], 0.3, 1.0, H), 2600.0, NAV, 0.12) }
fn bq_tab() -> Vec<f32> { sfx(&[n(0.0, 76.0, 0.15, 1.0, H)], 2600.0, NAV, 0.06) }
fn bq_type() -> Vec<f32> { sfx(&[n(0.0, 83.0, 0.04, 1.0, H)], 2600.0, TYPE, 0.015) }

// Lounge Keys: electric piano and vibraphone.
fn lk_update() -> Vec<f32> {
    let mut v = arp(0.0, 0.035, &[53.0, 64.0, 67.0, 69.0, 72.0], 2.0, 0.6, K);
    v.push(n(0.3, 84.0, 1.6, 0.35, Voice::Vibe));
    jingle(&v, 2600.0, 0.1)
}
fn lk_hover() -> Vec<f32> { sfx(&[n(0.0, 91.0, 0.08, 1.0, Voice::Vibe)], 3000.0, HOVER, 0.03) }
fn lk_click() -> Vec<f32> { sfx(&[n(0.0, 79.0, 0.15, 1.0, K)], 3000.0, CLICK, 0.06) }
fn lk_open() -> Vec<f32> { sfx(&arp(0.0, 0.03, &[65.0, 69.0, 72.0, 76.0], 0.4, 1.0, K), 3000.0, NAV, 0.15) }
fn lk_close() -> Vec<f32> { sfx(&arp(0.0, 0.04, &[76.0, 72.0, 69.0], 0.35, 1.0, K), 3000.0, NAV, 0.12) }
fn lk_tab() -> Vec<f32> { sfx(&arp(0.0, 0.05, &[84.0, 88.0], 0.25, 1.0, Voice::Vibe), 3000.0, NAV, 0.1) }
fn lk_type() -> Vec<f32> { sfx(&[n(0.0, 91.0, 0.04, 1.0, K)], 3000.0, TYPE, 0.015) }

// Island: steel drum and shaker.
const P: Voice = Voice::SteelPan;
fn island_update() -> Vec<f32> {
    let mut v = arp(0.0, 0.1, &[74.0, 78.0, 81.0, 86.0], 1.0, 0.8, P);
    v.extend([n(0.0, 0.0, 0.06, 0.3, Voice::Shaker), n(0.2, 0.0, 0.06, 0.3, Voice::Shaker)]);
    jingle(&v, 3200.0, 0.11)
}
fn island_hover() -> Vec<f32> { sfx(&[n(0.0, 93.0, 0.06, 1.0, P)], 3500.0, HOVER, 0.02) }
fn island_click() -> Vec<f32> { sfx(&[n(0.0, 86.0, 0.12, 1.0, P)], 3500.0, CLICK, 0.04) }
fn island_open() -> Vec<f32> { sfx(&arp(0.0, 0.06, &[79.0, 83.0, 86.0], 0.3, 1.0, P), 3500.0, NAV, 0.12) }
fn island_close() -> Vec<f32> { sfx(&arp(0.0, 0.06, &[86.0, 83.0, 79.0], 0.3, 1.0, P), 3500.0, NAV, 0.12) }
fn island_tab() -> Vec<f32> { sfx(&arp(0.0, 0.05, &[84.0, 88.0], 0.18, 1.0, P), 3500.0, NAV, 0.06) }
fn island_type() -> Vec<f32> { sfx(&[n(0.0, 0.0, 0.03, 1.0, Voice::Shaker)], 6000.0, TYPE, 0.01) }

// Crystal: celesta, glass and music-box bells.
const C: Voice = Voice::Celesta;
fn crystal_update() -> Vec<f32> {
    let mut v = vec![n(0.0, 72.0, 1.4, 0.6, Voice::Bell)];
    v.extend(arp(0.05, 0.05, &[84.0, 88.0, 91.0, 95.0], 1.2, 0.6, C));
    jingle(&v, 4500.0, 0.1)
}
fn crystal_hover() -> Vec<f32> { sfx(&[n(0.0, 96.0, 0.08, 1.0, Voice::Glass)], 5000.0, HOVER, 0.03) }
fn crystal_click() -> Vec<f32> { sfx(&[n(0.0, 91.0, 0.15, 1.0, C)], 5000.0, CLICK, 0.06) }
fn crystal_open() -> Vec<f32> { sfx(&arp(0.0, 0.04, &[84.0, 88.0, 91.0, 96.0], 0.4, 1.0, C), 5000.0, NAV, 0.15) }
fn crystal_close() -> Vec<f32> { sfx(&arp(0.0, 0.04, &[96.0, 91.0, 88.0, 84.0], 0.4, 1.0, C), 5000.0, NAV, 0.15) }
fn crystal_tab() -> Vec<f32> { sfx(&arp(0.0, 0.05, &[91.0, 96.0], 0.25, 1.0, Voice::Bell), 5000.0, NAV, 0.1) }
fn crystal_type() -> Vec<f32> { sfx(&[n(0.0, 100.0, 0.04, 1.0, Voice::Glass)], 5000.0, TYPE, 0.015) }

// Bubblegum: bouncy bloops that bend.
const T: Voice = Voice::Triangle;
fn gum_update() -> Vec<f32> {
    jingle(&[bend(0.0, 72.0, 0.12, 1.0, T, 5.0), bend(0.1, 79.0, 0.12, 1.0, T, 5.0), bend(0.2, 76.0, 0.12, 1.0, T, 5.0), bend(0.32, 84.0, 0.4, 1.0, T, 5.0)], 3000.0, 0.12)
}
fn gum_hover() -> Vec<f32> { sfx(&[bend(0.0, 88.0, 0.04, 1.0, T, 3.0)], 3500.0, HOVER, 0.015) }
fn gum_click() -> Vec<f32> { sfx(&[bend(0.0, 79.0, 0.07, 1.0, T, 5.0)], 3500.0, CLICK, 0.02) }
fn gum_open() -> Vec<f32> { sfx(&[bend(0.0, 72.0, 0.08, 1.0, T, 7.0), bend(0.07, 79.0, 0.12, 1.0, T, 7.0)], 3500.0, NAV, 0.05) }
fn gum_close() -> Vec<f32> { sfx(&[bend(0.0, 84.0, 0.08, 1.0, T, -7.0), bend(0.07, 77.0, 0.12, 1.0, T, -7.0)], 3500.0, NAV, 0.05) }
fn gum_tab() -> Vec<f32> { sfx(&[bend(0.0, 81.0, 0.06, 1.0, T, 4.0), bend(0.06, 86.0, 0.08, 1.0, T, 4.0)], 3500.0, NAV, 0.03) }
fn gum_type() -> Vec<f32> { sfx(&[bend(0.0, 91.0, 0.03, 1.0, T, 2.0)], 3500.0, TYPE, 0.01) }
