//! Optional background music while Obelisk is open (Settings → General → Music, off by default). Obelisk's own tracks
//! are original loops synthesized here with sound.rs's little band, so nothing copyrighted ships (user, 2026-11-06: no
//! Wii Shop theme). "My own music" plays the user's own files from a folder they pick; they never leave this PC.
//! The player itself lives in the UI (player.svelte.ts): this module renders the tracks and lets it stream the files.

use crate::sound::{m, render_loop, Note, Voice};

/// Obelisk's tracks, in the order Settings lists them: (setting id, synth). Lounge is the default.
pub const TRACKS: [(&str, fn() -> Vec<f32>); 2] = [("lounge", lounge), ("shopping", window_shopping)];

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
/// `bar` is the beats to a bar (4, or 3 for a waltz).
struct Song { bpm: f64, swing: f64, bar: f64, notes: Vec<Note> }

impl Song {
    fn new(bpm: f64, swing: f64) -> Self { Song { bpm, swing, bar: 4.0, notes: Vec::new() } }

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
        let mut beat = bar as f64 * self.bar;
        for tok in text.split_whitespace() {
            let (name, len) = tok.split_once('/').expect("note/beats");
            let len: f64 = len.parse().expect("beats");
            if name != "r" { self.play(beat, midi(name), len * 0.95, vol, voice) }
            beat += len;
        }
    }

    /// The chords, one or two to a bar: "Cmaj7 | Am7 | Em7 A7 | ...".
    fn chords(&self, text: &str) -> Vec<(f64, f64, Vec<f64>)> {
        let mut out = Vec::new();
        for (bar, cell) in text.split('|').enumerate() {
            let names: Vec<&str> = cell.split_whitespace().collect();
            let each = self.bar / names.len() as f64;
            for (i, name) in names.iter().enumerate() { out.push((bar as f64 * self.bar + i as f64 * each, each, chord(name))) }
        }
        out
    }

    fn render(&self, bars: usize, cutoff: f64) -> Vec<f32> { render_loop(&self.notes, self.secs(bars as f64 * self.bar), cutoff, 0.06) }
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
        "add9" => &[0.0, 4.0, 7.0, 14.0], "maj9" => &[0.0, 4.0, 7.0, 11.0, 14.0], "m9" => &[0.0, 3.0, 7.0, 10.0, 14.0],
        _ => panic!("chord {name}"),
    };
    shape.iter().map(|i| 60.0 + pc + i).collect()
}

/// An easy swung stroll with soft keys, round bass, brushes and a marimba tune.
fn window_shopping() -> Vec<f32> {
    let mut s = Song::new(100.0, 0.62);
    let chords = s.chords("Cmaj7 | Am7 | Dm7 | G7 | Em7 | A7 | Dm7 | G7 | Fmaj7 | Fm6 | Em7 | A7 | Dm7 | G7 | Cmaj7 | G7");
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

/// A little shop lounge: bossa guitar on the off-beats, a bass stepping root to fifth and a shaker, over 9th chords
/// with the sixth leaning into the second (VI7 to ii) that shop music loves. No tune on top (user, 2026-11-06: the
/// vibraphone lead was too bright; the bare groove is the one they liked).
fn lounge() -> Vec<f32> {
    let mut s = Song::new(150.0, 0.5);
    let chords = s.chords("Bbmaj9 | G7 | Cm9 | F7 | Dm7 | G7 | Cm9 | F7 | Ebmaj9 | Ebm6 | Dm7 | G7 | Cm9 | F7 | Bbmaj9 | F7");
    for (bar, (beat, _, notes)) in chords.iter().enumerate() {
        let (root, fifth) = (notes[0] - 24.0, notes[0] - 17.0);
        for (off, k, l) in [(0.0, root, 1.4), (1.5, fifth, 0.45), (2.0, fifth, 1.4), (3.5, root, 0.45)] { s.play(beat + off, k, l, 0.6, Voice::Bass) }
        let slots: &[f64] = if bar % 2 == 0 { &[0.0, 1.5, 3.0] } else { &[1.0, 2.5] };
        for &off in slots { for &k in &notes[1..] { s.play(beat + off, k, 0.45, 0.16, Voice::Pluck) } }
        for e in 0..8 { s.play(beat + e as f64 * 0.5, 0.0, 0.05, if e % 2 == 1 { 0.12 } else { 0.06 }, Voice::Shaker) }
    }
    s.render(16, 4500.0)
}
