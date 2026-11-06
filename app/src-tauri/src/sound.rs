//! Obelisk's sounds: the little jingle that plays on the first start after an update (user, 2026-11-06: "nothing loud
//! ... cute and Nintendo-y"; it plays after every update, so each is short and soft, never a beepy 8-bit tune). Everything is synthesized here and played from memory through winmm, so no audio files
//! ship and every tune is original. Settings → General picks one of the three, a random one each time, or none.

use std::f64::consts::{PI, TAU};

const RATE: u32 = 32_000;

/// The jingles, in the order Settings lists them: (setting id, synth).
pub const JINGLES: [(&str, fn() -> Vec<f32>); 3] = [("glow", glow), ("bubbles", bubbles), ("marimba", marimba)];

/// Plays the jingle the user picked, after an update installed. Settings key `updateJingle`: an id, "random" or "off".
pub fn update_jingle(app: &tauri::AppHandle) {
    let choice = crate::settings_value(app).get("updateJingle").and_then(|v| v.as_str()).unwrap_or("off").to_string(); // staged: silent until the user picks one
    if let Some(pcm) = pick(&choice) { std::thread::spawn(move || play(&pcm)); }
}

/// Settings' preview button (also callable from the control API): plays one jingle now.
#[tauri::command]
pub fn jingle_preview(name: String) -> Result<(), String> {
    let pcm = pick(&name).ok_or_else(|| format!("No jingle called \"{name}\""))?;
    std::thread::spawn(move || play(&pcm));
    Ok(())
}

fn pick(choice: &str) -> Option<Vec<f32>> {
    let synth = match choice {
        "off" => return None,
        "random" => {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
            JINGLES[nanos as usize % JINGLES.len()].1
        }
        id => JINGLES.iter().find(|j| j.0 == id)?.1,
    };
    Some(synth())
}

/// A mono 16-bit WAV of the samples, as PlaySoundW (and any player) reads it.
pub fn wav(pcm: &[f32]) -> Vec<u8> {
    let data = (pcm.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data as usize);
    wav.extend(b"RIFF"); wav.extend((36 + data).to_le_bytes()); wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes()); wav.extend(1u16.to_le_bytes()); wav.extend(1u16.to_le_bytes());
    wav.extend(RATE.to_le_bytes()); wav.extend((RATE * 2).to_le_bytes()); wav.extend(2u16.to_le_bytes()); wav.extend(16u16.to_le_bytes());
    wav.extend(b"data"); wav.extend(data.to_le_bytes());
    for &v in pcm { wav.extend(((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()) }
    wav
}

#[cfg(windows)]
pub(crate) fn play(pcm: &[f32]) {
    #[link(name = "winmm")]
    unsafe extern "system" { fn PlaySoundW(sound: *const u8, module: isize, flags: u32) -> i32; }
    let wav = wav(pcm);
    // SND_MEMORY | SND_NODEFAULT, played to the end on this thread (the buffer has to live that long). A newer sound
    // cuts this one short, so pressing preview again just restarts.
    unsafe { PlaySoundW(wav.as_ptr(), 0, 0x0004 | 0x0002) };
}

#[cfg(not(windows))]
pub(crate) fn play(_pcm: &[f32]) {}

// ---- the synth ----

/// MIDI note number to Hz.
pub(crate) fn m(n: f64) -> f64 { 440.0 * 2f64.powf((n - 69.0) / 12.0) }

#[derive(Clone, Copy)]
pub(crate) enum Voice {
    /// Plain sine with a quick fade: a sparkle on top.
    Sine,
    /// Soft triangle that pops: bubbles.
    Triangle,
    /// Sine with fast-dying overtones: a little wooden marimba.
    Wood,
    /// Three detuned saws that swell in and fade: a warm glow.
    Pad,
    // The background music's band (music.rs):
    /// Soft FM electric piano.
    Keys,
    /// Round triangle bass.
    Bass,
    /// A soft brushed tick.
    Hat,
    /// A soft, low thump.
    Kick,
    /// A plucked string (nylon guitar, ukulele): the higher harmonics die first.
    Pluck,
    /// A soft shaker shake.
    Shaker,
    /// A glassy sine with a faint fifth and octave above: the social cues' voice (cues.rs).
    Glass,
}

pub(crate) struct Note { pub at: f64, pub hz: f64, pub len: f64, pub vol: f64, pub voice: Voice, pub slide: f64 }

/// A note at `at` seconds, `len` long, on `voice`.
pub(crate) fn n(at: f64, note: f64, len: f64, vol: f64, voice: Voice) -> Note { Note { at, hz: m(note), len, vol, voice, slide: 0.0 } }

impl Voice {
    /// One sample at phase `ph` (cycles since the note began), `t` seconds into the note.
    fn sample(self, ph: f64, t: f64) -> f64 {
        match self {
            Voice::Sine => (TAU * ph).sin() * (-t * 9.0).exp(),
            Voice::Triangle => (4.0 * (ph.fract() - 0.5).abs() - 1.0) * (-t * 9.0).exp(),
            Voice::Wood => {
                (TAU * ph).sin() * (-t * 11.0).exp() + 0.25 * (TAU * 4.0 * ph).sin() * (-t * 45.0).exp()
                    + 0.08 * (TAU * 9.8 * ph).sin() * (-t * 120.0).exp()
            }
            Voice::Pad => {
                let saw = |p: f64| 2.0 * p.fract() - 1.0;
                let swell = (PI / 2.0 * (t / 0.42).min(1.0)).sin().powi(2) * if t > 0.42 { (-(t - 0.42) * 2.4).exp() } else { 1.0 };
                (saw(ph) + saw(ph * 1.0035) + saw(ph * 0.9965)) / 3.0 * swell
            }
            Voice::Keys => (TAU * ph + 0.9 * (-t * 4.0).exp() * (TAU * ph).sin()).sin() * (-t * 1.6).exp(),
            Voice::Bass => (4.0 * (ph.fract() - 0.5).abs() - 1.0) * (-t * 2.2).exp(),
            Voice::Hat => {
                // A cheap white noise from the sample index (xorshift-style hash).
                let mut x = (t * RATE as f64) as u64 ^ 0x9E37_79B9_7F4A_7C15;
                x ^= x << 13; x ^= x >> 7; x ^= x << 17;
                ((x % 2001) as f64 / 1000.0 - 1.0) * (-t * 70.0).exp()
            }
            Voice::Pluck => (1..=6).map(|k| (TAU * k as f64 * ph).sin() / k as f64 * (-t * (2.5 + 2.0 * k as f64)).exp()).sum::<f64>() * 0.7,
            Voice::Shaker => {
                let mut x = (t * RATE as f64) as u64 ^ 0xD1B5_4A32_D192_ED03;
                x ^= x << 13; x ^= x >> 7; x ^= x << 17;
                ((x % 2001) as f64 / 1000.0 - 1.0) * (t / 0.012).min(1.0) * (-t * 40.0).exp()
            }
            Voice::Glass => {
                ((TAU * ph).sin() + 0.22 * (TAU * 2.0 * ph).sin() * (-t * 25.0).exp() + 0.1 * (TAU * 3.0 * ph).sin() * (-t * 40.0).exp())
                    * (-t * 12.0).exp()
            }
            Voice::Kick => (TAU * (55.0 * t + 6.0 * (1.0 - (-t * 30.0).exp()))).sin() * (-t * 10.0).exp(),
        }
    }
}

/// Mixes the notes, softens them through a one-pole low-pass at `cutoff` Hz, and scales the loudest moment to `peak`
/// (all of them stay quiet: about a tenth of full scale).
pub(crate) fn render(notes: &[Note], cutoff: f64, peak: f32) -> Vec<f32> {
    let end = notes.iter().map(|n| n.at + n.len).fold(0.0, f64::max);
    let mut pcm = vec![0f32; (end * RATE as f64) as usize + 1];
    mix(notes, &mut pcm);
    let pcm = low_pass(pcm, cutoff, false);
    let loudest = pcm.iter().fold(0f32, |m, s| m.max(s.abs())).max(1e-6);
    pcm.into_iter().map(|s| s * peak / loudest).collect()
}

/// The same for a seamless loop `secs` long (what rings past the end wraps round onto the start), set to an even
/// loudness of `rms` so every track sits at the same level under the volume slider.
pub(crate) fn render_loop(notes: &[Note], secs: f64, cutoff: f64, rms: f32) -> Vec<f32> {
    let mut pcm = vec![0f32; (secs * RATE as f64) as usize];
    mix(notes, &mut pcm);
    let pcm = low_pass(pcm, cutoff, true);
    let now = (pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len() as f32).sqrt().max(1e-6);
    let loudest = pcm.iter().fold(0f32, |m, s| m.max(s.abs())).max(1e-6);
    let gain = (rms / now).min(0.9 / loudest);
    pcm.into_iter().map(|s| s * gain).collect()
}

fn mix(notes: &[Note], pcm: &mut [f32]) {
    let rate = RATE as f64;
    let frames = pcm.len();
    for note in notes {
        let start = (note.at * rate) as usize;
        let mut ph = 0.0;
        for k in 0..(note.len * rate) as usize {
            let t = k as f64 / rate;
            // Slides glide up `slide` semitones over the first 35 ms; held voices drift with a slow gentle vibrato.
            let mut hz = note.hz * 2f64.powf(note.slide * (t / 0.035).min(1.0) / 12.0);
            if let Voice::Pad = note.voice { hz *= 1.0 + 0.003 * (TAU * 5.0 * t).sin() }
            ph += hz / rate;
            let edges = (t * 400.0).min(1.0) * ((note.len - t) * 200.0).min(1.0); // no clicks at either end
            pcm[(start + k) % frames] += (note.voice.sample(ph, t) * note.vol * edges) as f32;
        }
    }
}

/// A one-pole low-pass at `cutoff` Hz that takes the edge off. A loop starts the filter from its own end, so the seam
/// doesn't click.
fn low_pass(mut pcm: Vec<f32>, cutoff: f64, looped: bool) -> Vec<f32> {
    let a = (1.0 - (-TAU * cutoff / RATE as f64).exp()) as f32;
    let mut y = 0.0f32;
    if looped { for &s in &pcm[pcm.len().saturating_sub(RATE as usize)..] { y += a * (s - y) } }
    for s in pcm.iter_mut() { y += a * (*s - y); *s = y }
    pcm
}

/// Four round little bloops that each slide up as they pop, with a hop before the last, higher one.
fn bubbles() -> Vec<f32> {
    let bloop = |at, note, len| Note { slide: 5.0, ..n(at, note, len, 1.0, Voice::Triangle) };
    let notes = [bloop(0.0, 69.0, 0.12), bloop(0.1, 74.0, 0.12), bloop(0.2, 71.0, 0.14), bloop(0.38, 78.0, 0.4)];
    render(&notes, 3000.0, 0.12)
}

/// Wooden mallets skipping down in triplets, a breath, then a hop up to a low-doubled C.
fn marimba() -> Vec<f32> {
    let notes = [
        n(0.0, 79.0, 0.3, 1.0, Voice::Wood), n(0.09, 77.0, 0.3, 0.9, Voice::Wood), n(0.18, 76.0, 0.3, 0.9, Voice::Wood),
        n(0.36, 79.0, 0.3, 0.9, Voice::Wood), n(0.45, 84.0, 0.5, 1.0, Voice::Wood), n(0.45, 72.0, 0.5, 0.6, Voice::Wood),
    ];
    render(&notes, 4000.0, 0.13)
}

/// A soft warm chord (Fmaj7) that swells in and fades, with one faint chime at its height.
fn glow() -> Vec<f32> {
    let mut notes: Vec<Note> = [53.0, 65.0, 69.0, 72.0, 76.0].iter().map(|&k| n(0.0, k, 1.25, if k < 60.0 { 0.6 } else { 0.8 }, Voice::Pad)).collect();
    notes.push(n(0.4, 81.0, 0.6, 0.25, Voice::Sine));
    render(&notes, 1400.0, 0.08)
}
