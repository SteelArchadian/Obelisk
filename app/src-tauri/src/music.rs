//! Optional background music while Obelisk is open (Settings → General → Music, off by default). Obelisk's own tracks
//! are original loops synthesized here with sound.rs's little band, so nothing copyrighted ships (user, 2026-11-06: no
//! Wii Shop theme). "My own music" plays the user's own files from a folder they pick; they never leave this PC.
//! The player itself lives in the UI (player.svelte.ts): this module renders the tracks and lets it stream the files.

use crate::sound::{m, render_loop, Note, Voice};

/// Obelisk's tracks, in the order Settings lists them: (setting id, synth).
pub const TRACKS: [(&str, fn() -> Vec<f32>); 3] = [("shopping", window_shopping), ("lantern", lantern), ("garden", pocket_garden)];

/// Audio files the UI can play from the user's music folder.
const AUDIO: [&str; 7] = ["mp3", "ogg", "oga", "opus", "wav", "flac", "m4a"];

/// One of Obelisk's tracks as a WAV file the UI's player can stream (rendered once per run into the data folder's
/// music cache, so a newer Obelisk always plays its newer tracks).
#[tauri::command]
pub async fn music_track(app: tauri::AppHandle, id: String) -> Result<String, String> {
    use tauri::Manager;
    static DONE: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let synth = TRACKS.iter().find(|t| t.0 == id).ok_or_else(|| format!("No track called \"{id}\""))?.1;
    let dir = crate::datadir::root().join("music");
    let path = dir.join(format!("{id}.wav"));
    if !DONE.lock().unwrap().contains(&id) {
        let pcm = tauri::async_runtime::spawn_blocking(synth).await.map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(&path, crate::sound::wav(&pcm)).map_err(|e| e.to_string())?;
        DONE.lock().unwrap().push(id);
    }
    app.asset_protocol_scope().allow_directory(&dir, false).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// The audio files directly in the user's music `folder`, sorted by name, which the player may then stream.
#[tauri::command]
pub fn music_files(app: tauri::AppHandle, folder: String) -> Result<Vec<String>, String> {
    use tauri::Manager;
    let mut files: Vec<String> = std::fs::read_dir(&folder).map_err(|e| format!("Couldn't open {folder}: {e}"))?
        .filter_map(|e| e.ok()).map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|x| x.to_str()).is_some_and(|x| AUDIO.contains(&x.to_ascii_lowercase().as_str())))
        .filter_map(|p| p.to_str().map(String::from))
        .collect();
    files.sort_by_key(|f| f.to_lowercase());
    app.asset_protocol_scope().allow_directory(&folder, false).map_err(|e| e.to_string())?;
    Ok(files)
}

// ---- the tracks ----

/// A song being written: notes go in by beat, and come out in seconds (with swung eighths if `swing` > 0.5).
struct Song { bpm: f64, swing: f64, notes: Vec<Note> }

impl Song {
    fn new(bpm: f64, swing: f64) -> Self { Song { bpm, swing, notes: Vec::new() } }

    fn secs(&self, beat: f64) -> f64 {
        let whole = beat.floor();
        let f = if (beat - whole - 0.5).abs() < 1e-9 { self.swing } else { beat - whole };
        (whole + f) * 60.0 / self.bpm
    }

    fn play(&mut self, beat: f64, note: f64, beats: f64, vol: f64, voice: Voice) {
        let at = self.secs(beat);
        let len = self.secs(beat + beats) - at;
        self.notes.push(Note { at, hz: m(note), len, vol, voice, slide: 0.0 });
    }

    /// A melody from bar `bar` on: "e5/1.5 g5/.5 r/1 ..." (note or r for a rest, then its length in beats).
    fn line(&mut self, bar: usize, text: &str, vol: f64, voice: Voice) {
        let mut beat = bar as f64 * 4.0;
        for tok in text.split_whitespace() {
            let (name, len) = tok.split_once('/').expect("note/beats");
            let len: f64 = len.parse().expect("beats");
            if name != "r" { self.play(beat, midi(name), len * 0.95, vol, voice) }
            beat += len;
        }
    }

    /// The chords, one or two to a bar: "Cmaj7 | Am7 | Em7 A7 | ...".
    fn chords(text: &str) -> Vec<(f64, f64, Vec<f64>)> {
        let mut out = Vec::new();
        for (bar, cell) in text.split('|').enumerate() {
            let names: Vec<&str> = cell.split_whitespace().collect();
            let each = 4.0 / names.len() as f64;
            for (i, name) in names.iter().enumerate() { out.push((bar as f64 * 4.0 + i as f64 * each, each, chord(name))) }
        }
        out
    }

    fn render(&self, bars: usize, cutoff: f64) -> Vec<f32> { render_loop(&self.notes, self.secs(bars as f64 * 4.0), cutoff, 0.06) }
}

/// "c#5" → MIDI 73.
fn midi(name: &str) -> f64 {
    let b = name.as_bytes();
    let pc = match b[0] { b'c' => 0, b'd' => 2, b'e' => 4, b'f' => 5, b'g' => 7, b'a' => 9, b'b' => 11, _ => panic!("note {name}") };
    let (acc, oct) = match b[1] { b'#' => (1, &name[2..]), b'b' if b.len() > 2 => (-1, &name[2..]), _ => (0, &name[1..]) };
    (12 * (oct.parse::<i32>().expect("octave") + 1) + pc + acc) as f64
}

/// A chord symbol as MIDI notes around middle C, root first.
fn chord(name: &str) -> Vec<f64> {
    let (root, rest) = if name.len() > 1 && matches!(&name[1..2], "#" | "b") { name.split_at(2) } else { name.split_at(1) };
    let mut pc = midi(&format!("{}4", root.to_lowercase())) - 60.0;
    if pc > 6.0 { pc -= 12.0 }
    let shape: &[f64] = match rest {
        "" => &[0.0, 4.0, 7.0], "m" => &[0.0, 3.0, 7.0], "7" => &[0.0, 4.0, 7.0, 10.0], "maj7" => &[0.0, 4.0, 7.0, 11.0],
        "m7" => &[0.0, 3.0, 7.0, 10.0], "m6" => &[0.0, 3.0, 7.0, 9.0], "sus4" => &[0.0, 5.0, 7.0], "6" => &[0.0, 4.0, 7.0, 9.0],
        _ => panic!("chord {name}"),
    };
    shape.iter().map(|i| 60.0 + pc + i).collect()
}

/// An easy swung stroll with soft keys, round bass, brushes and a marimba tune.
fn window_shopping() -> Vec<f32> {
    let mut s = Song::new(100.0, 0.62);
    let chords = Song::chords("Cmaj7 | Am7 | Dm7 | G7 | Em7 | A7 | Dm7 | G7 | Fmaj7 | Fm6 | Em7 | A7 | Dm7 | G7 | Cmaj7 | G7");
    for (beat, len, notes) in &chords {
        let stabs: &[(f64, f64)] = if *len >= 4.0 { &[(0.0, 1.4), (2.5, 1.0)] } else { &[(0.0, 1.4)] };
        for &(off, l) in stabs { for &k in &notes[1..] { s.play(beat + off, k, l, 0.22, Voice::Keys) } }
        s.play(*beat, notes[0] - 24.0, 1.8, 0.7, Voice::Bass);
        if *len >= 4.0 { s.play(beat + 2.0, notes[0] - 17.0, 1.8, 0.6, Voice::Bass) }
    }
    for beat in 0..64 {
        s.play(beat as f64 + 0.5, 0.0, 0.1, 0.12, Voice::Hat);
        if beat % 2 == 0 { s.play(beat as f64, 0.0, 0.3, 0.35, Voice::Kick) }
    }
    s.line(0, "e5/1.5 g5/.5 b5/1 a5/1 g5/2 e5/.5 c5/.5 e5/1 f5/1 a5/.5 g5/.5 f5/1 e5/1 d5/3 r/1 \
               g5/1 b5/1 d6/1.5 b5/.5 c#6/2 a5/1 g5/1 f5/.5 e5/.5 f5/1 a5/1 c6/1 b5/3 r/1", 0.7, Voice::Wood);
    s.line(8, "a5/1.5 c6/.5 e6/2 d6/1 g#5/1 c6/2 b5/1 g5/.5 e5/.5 g5/2 c#6/1 e6/1 g5/1 a5/1 \
               f5/1 a5/1 d6/1 c6/1 b5/1 g5/1 f5/1 d5/1 e5/2 d5/.5 e5/.5 g5/1 r/2 d5/.5 f5/.5 g5/1", 0.7, Voice::Wood);
    s.render(16, 4000.0)
}

/// Slow and calm: a held warm chord, a music box turning under a soft flute.
fn lantern() -> Vec<f32> {
    let mut s = Song::new(72.0, 0.5);
    let chords = Song::chords("Cmaj7 | Am7 | Fmaj7 | Gsus4 G | Em7 | Am7 | Dm7 | G");
    for (beat, len, notes) in &chords {
        for &k in &notes[1..] { s.play(*beat, k, *len, 0.16, Voice::Warm) }
        s.play(*beat, notes[0] - 24.0, *len, 0.6, Voice::Bass);
        // The music box walks up and back down the chord in eighths, an octave up.
        let tones: Vec<f64> = notes.iter().map(|k| k + 12.0).collect();
        let order = [0, 1, 2, tones.len() - 1, 2, 1, 0, 1];
        for (i, &j) in order.iter().take((*len * 2.0) as usize).enumerate() { s.play(beat + i as f64 * 0.5, tones[j], 0.8, 0.22, Voice::Bell) }
    }
    s.line(0, "e5/3 d5/1 c5/2 e5/2 a5/3 g5/1 g5/4 b5/2 g5/2 e5/3 c5/1 d5/2 f5/1 e5/1 d5/4", 0.5, Voice::Flute);
    s.render(8, 3000.0)
}

/// Bouncy and bright: a kalimba tune over a hopping bass and soft wooden off-beats.
fn pocket_garden() -> Vec<f32> {
    let mut s = Song::new(120.0, 0.5);
    let chords = Song::chords("G | Em | C | D | G | Bm | C | D7 | C | D | Bm | Em | Am | D | G | D");
    for (beat, len, notes) in &chords {
        for e in 0..(*len * 2.0) as usize {
            let b = beat + e as f64 * 0.5;
            s.play(b, notes[0] - 24.0 + if e % 2 == 1 { 12.0 } else { 0.0 }, 0.45, 0.5, Voice::Bass);
            if e % 2 == 1 { for &k in &notes[1..] { s.play(b, k, 0.4, 0.16, Voice::Wood) } }
        }
    }
    for beat in 0..64 {
        s.play(beat as f64 + 0.5, 0.0, 0.08, 0.1, Voice::Hat);
        if beat % 2 == 0 { s.play(beat as f64, 0.0, 0.3, 0.4, Voice::Kick) }
    }
    s.line(0, "b4/.5 d5/.5 g5/1 f#5/.5 g5/.5 a5/1 g5/1.5 e5/.5 b4/1 e5/1 e5/.5 g5/.5 c6/1 b5/.5 a5/.5 g5/1 a5/3 r/1 \
               b5/.5 a5/.5 g5/.5 d5/.5 g5/1 b5/1 d6/1.5 c#6/.5 b5/1 f#5/1 e5/.5 f#5/.5 g5/.5 a5/.5 b5/1 c6/1 a5/2 f#5/1 r/1", 0.8, Voice::Kalimba);
    s.line(8, "e5/1 g5/.5 e5/.5 c6/2 d6/1 a5/.5 f#5/.5 a5/2 b5/1 d6/1 f#6/1 d6/1 e6/3 r/1 \
               c6/1 a5/1 e5/1 a5/1 f#5/1 a5/1 d6/1 c6/1 b5/1.5 a5/.5 g5/1 d5/1 f#5/.5 g5/.5 a5/1 d5/1 r/1", 0.8, Voice::Kalimba);
    s.render(16, 4500.0)
}
