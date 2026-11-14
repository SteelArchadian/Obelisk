//! Optional background music while Obelisk is open (Settings → General → Music, off by default). Obelisk's own tracks
//! are original loops synthesized here with sound.rs's little band, so nothing copyrighted ships (user, 2026-11-06: no
//! Wii Shop theme). "My own music" plays the user's own files from a folder they pick; they never leave this PC.
//! The player itself lives in the UI (player.svelte.ts): this module renders the tracks and lets it stream the files.

use crate::sound::{m, render_loop, Note, Voice};

/// Obelisk's tracks, in the order Settings lists them: (setting id, synth). Lounge is the default. The last four are
/// the sound themes' own music (Rainy Day and Boutique use Puddle and Boutique).
pub const TRACKS: [(&str, fn() -> Vec<f32>); 15] = [
    ("lounge", lounge), ("shopping", window_shopping), ("puddle", puddle), ("boutique", boutique), ("drizzle", drizzle),
    ("after-hours", after_hours), ("after-hours-harp", after_hours_harp), ("after-hours-keys", after_hours_keys),
    ("velvet", velvet), ("starlit", starlit_soft), ("breeze", breeze), ("lounge-keys", lounge_keys), ("island", island),
    ("crystal", crystal), ("bubblegum", bubblegum),
];

/// Tracks that ship as recordings, not synths (Night Kiss is composed from the CC0 VSCO and FreePats samples, so it is
/// ours to ship): (setting id, m4a bytes). Piano, clarinet, horn, cello and string pad; it loops cleanly because it
/// ends at the level it starts and on the home chord.
const BUNDLED: [(&str, &[u8]); 1] = [("night-kiss", include_bytes!("../music/night-kiss.m4a"))];

/// Audio files the UI can play from the user's music folder.
const AUDIO: [&str; 7] = ["mp3", "ogg", "oga", "opus", "wav", "flac", "m4a"];

/// One of Obelisk's tracks as a WAV file the UI's player can stream (rendered once per run into the data folder's
/// music cache, so a newer Obelisk always plays its newer tracks).
#[tauri::command]
pub async fn music_track(app: tauri::AppHandle, id: String) -> Result<String, String> {
    use tauri::Manager;
    static DONE: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    if let Some((_, bytes)) = BUNDLED.iter().find(|t| t.0 == id) {
        let dir = crate::datadir::root().join("music");
        let path = dir.join(format!("{id}.m4a"));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        app.asset_protocol_scope().allow_directory(&dir, false).map_err(|e| e.to_string())?;
        return Ok(path.to_string_lossy().into_owned());
    }
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
pub async fn music_files(app: tauri::AppHandle, folder: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || list(&app, &folder)).await.map_err(|e| e.to_string())?
}

fn list(app: &tauri::AppHandle, folder: &str) -> Result<Vec<String>, String> {
    use tauri::Manager;
    let mut files: Vec<String> = std::fs::read_dir(&folder).map_err(|e| format!("Couldn't open {folder}: {e}"))?
        .filter_map(|e| e.ok()).map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|x| x.to_str()).is_some_and(|x| AUDIO.contains(&x.to_ascii_lowercase().as_str())))
        .filter_map(|p| p.to_str().map(String::from))
        .collect();
    files.sort_by_key(|f| f.to_lowercase());
    app.asset_protocol_scope().allow_directory(folder, false).map_err(|e| e.to_string())?;
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

/// A shaker on every eighth, the off-beats louder.
fn shaker(s: &mut Song, beat: f64, eighths: usize) {
    for e in 0..eighths { s.play(beat + e as f64 * 0.5, 0.0, 0.05, if e % 2 == 1 { 0.1 } else { 0.05 }, Voice::Shaker) }
}

/// A rainy-window afternoon: slow electric-piano chords, a soft kick and brush, round bass. No tune.
fn puddle() -> Vec<f32> {
    let mut s = Song::new(84.0, 0.6);
    let chords = s.chords("Dmaj9 | Bm9 | Gmaj9 | A7 | F#m7 | Bm9 | Em9 | A7 | Dmaj9 | Bm9 | Gmaj9 | Gm6 | F#m7 | Bm9 | Em9 | A7");
    for (beat, _, notes) in &chords {
        for &k in &notes[1..] { s.play(*beat, k - 12.0, 3.6, 0.16, Voice::Keys) }
        s.play(beat + 2.5, notes[2] - 12.0, 1.2, 0.1, Voice::Keys);
        s.play(*beat, notes[0] - 24.0, 1.8, 0.65, Voice::Bass);
        s.play(beat + 2.0, notes[0] - 17.0, 1.6, 0.5, Voice::Bass);
        for q in [0.0, 2.5] { s.play(beat + q, 0.0, 0.3, 0.3, Voice::Kick) }
        for q in [1.0, 3.0] { s.play(beat + q, 0.0, 0.12, 0.1, Voice::Hat) }
    }
    s.render(16, 3200.0)
}

/// A little boutique: a harp turning over bossa chords, bass and shaker. No tune.
fn boutique() -> Vec<f32> {
    let mut s = Song::new(120.0, 0.5);
    let chords = s.chords("Gmaj9 | Em9 | Am9 | D7 | Bm7 | E7 | Am9 | D7 | Cmaj9 | Cm6 | Bm7 | E7 | Am9 | D7 | Gmaj9 | D7");
    for (beat, _, notes) in &chords {
        for (e, i) in [0, 1, 2, 3, 2, 1, 3, 2].into_iter().enumerate() { s.play(beat + e as f64 * 0.5, notes[i % notes.len()], 1.0, 0.2, Voice::Harp) }
        for (off, k, l) in [(0.0, notes[0] - 24.0, 1.4), (1.5, notes[0] - 17.0, 0.45), (2.0, notes[0] - 17.0, 1.4), (3.5, notes[0] - 24.0, 0.45)] {
            s.play(beat + off, k, l, 0.6, Voice::Bass)
        }
        shaker(&mut s, *beat, 8);
    }
    s.render(16, 3600.0)
}

/// After hours: the same bossa guitar and bass as Lounge, in A minor, over a faint warm pad. No tune on top.
fn after_hours() -> Vec<f32> {
    let mut s = Song::new(132.0, 0.5);
    let chords = s.chords("Am9 | Dm9 | G7 | Cmaj9 | Fmaj9 | Dm7 | E7 | Am9 | Am9 | Dm9 | G7 | Cmaj9 | Fmaj9 | Dm9 | E7 | E7");
    for (bar, (beat, _, notes)) in chords.iter().enumerate() {
        let (root, fifth) = (notes[0] - 24.0, notes[0] - 17.0);
        for (off, k, l) in [(0.0, root, 1.4), (1.5, fifth, 0.45), (2.0, fifth, 1.4), (3.5, root, 0.45)] { s.play(beat + off, k, l, 0.6, Voice::Bass) }
        let slots: &[f64] = if bar % 2 == 0 { &[0.0, 1.5, 3.0] } else { &[1.0, 2.5] };
        for &off in slots { for &k in &notes[1..] { s.play(beat + off, k, 0.45, 0.16, Voice::Pluck) } }
        for &k in &notes[1..] { s.play(*beat, k, 4.0, 0.04, Voice::Warm) }
        for e in 0..8 { s.play(beat + e as f64 * 0.5, 0.0, 0.05, if e % 2 == 1 { 0.1 } else { 0.05 }, Voice::Shaker) }
    }
    s.render(16, 3800.0)
}

/// A sea breeze: a steel drum singing a gentle tune in the middle of its range over guitar, bass and shaker.
fn breeze() -> Vec<f32> {
    let mut s = Song::new(104.0, 0.55);
    let chords = s.chords("Fmaj7 | Gm7 | Am7 | Bb6 | Fmaj7 | Dm7 | Gm7 | C7 | Bbmaj7 | Am7 | Gm7 | C7 | Fmaj7 | Dm7 | Gm7 C7 | Fmaj7");
    for (beat, len, notes) in &chords {
        s.play(*beat, notes[0] - 24.0, 1.4, 0.6, Voice::Bass);
        if *len >= 4.0 { s.play(beat + 2.0, notes[0] - 17.0, 1.4, 0.5, Voice::Bass) }
        for e in 0..(*len as usize) { for &k in &notes[1..] { s.play(beat + e as f64 + 0.5, k, 0.4, 0.12, Voice::Pluck) } }
        for e in 0..(*len as usize * 2) { s.play(beat + e as f64 * 0.5, 0.0, 0.05, if e % 2 == 1 { 0.1 } else { 0.05 }, Voice::Shaker) }
    }
    s.line(0, "c5/1 f5/1 a5/2 bb5/1 a5/.5 g5/.5 d5/2 e5/1 g5/1 c6/2 bb5/1 g5/1 d5/2 \
               a5/1.5 g5/.5 f5/1 c5/1 d5/1 f5/1 a5/2 g5/1 f5/.5 d5/.5 bb4/2 c5/3 r/1 \
               d5/1 f5/1 a5/1.5 c6/.5 c6/1 a5/1 e5/2 f5/1 g5/1 bb5/1 d6/1 c6/2 bb5/1 g5/1 \
               a5/1 c6/1 a5/1 f5/1 d5/1 e5/1 f5/1 a5/1 g5/1.5 f5/.5 e5/1 g5/1 f5/3 r/1", 0.35, Voice::SteelPan);
    s.render(16, 3500.0)
}

/// The bossa bass on beats 1, the and-of-2, 3 and the and-of-4.
fn bossa_bass(s: &mut Song, beat: f64, root: f64) {
    for (off, k, l) in [(0.0, root - 24.0, 1.4), (1.5, root - 17.0, 0.45), (2.0, root - 17.0, 1.4), (3.5, root - 24.0, 0.45)] {
        s.play(beat + off, k, l, 0.6, Voice::Bass)
    }
}

/// After Hours with Puddle's electric piano in place of the guitar: long, low chords and a soft answer each bar.
fn after_hours_keys() -> Vec<f32> {
    let mut s = Song::new(120.0, 0.5);
    let chords = s.chords("Am9 | Dm9 | G7 | Cmaj9 | Fmaj9 | Dm7 | E7 | Am9 | Am9 | Dm9 | G7 | Cmaj9 | Fmaj9 | Dm9 | E7 | E7");
    for (beat, _, notes) in &chords {
        for &k in &notes[1..] { s.play(*beat, k - 12.0, 2.8, 0.14, Voice::Keys) }
        for &k in &notes[2..] { s.play(beat + 3.0, k - 12.0, 0.8, 0.08, Voice::Keys) }
        bossa_bass(&mut s, *beat, notes[0]);
        shaker(&mut s, *beat, 8);
    }
    s.render(16, 3400.0)
}

/// After Hours with Boutique's harp turning over the chords instead of the strummed guitar.
fn after_hours_harp() -> Vec<f32> {
    let mut s = Song::new(120.0, 0.5);
    let chords = s.chords("Am9 | Dm9 | G7 | Cmaj9 | Fmaj9 | Dm7 | E7 | Am9 | Am9 | Dm9 | G7 | Cmaj9 | Fmaj9 | Dm9 | E7 | E7");
    for (beat, _, notes) in &chords {
        for (e, i) in [0, 2, 1, 3, 2, 1, 3, 1].into_iter().enumerate() { s.play(beat + e as f64 * 0.5, notes[i % notes.len()] - 12.0, 1.0, 0.2, Voice::Harp) }
        bossa_bass(&mut s, *beat, notes[0]);
        shaker(&mut s, *beat, 8);
    }
    s.render(16, 3400.0)
}

/// Starlit with the celesta halved, an octave lower, on every beat instead of every half beat, and no pad.
fn starlit_soft() -> Vec<f32> {
    let mut s = Song::new(80.0, 0.5);
    let chords = s.chords("Ebmaj7 | Cm7 | Abmaj7 | Bbsus4 Bb | Ebmaj7 | Gm7 | Abmaj7 | Bbsus4 Bb | Cm7 | Abmaj7 | Fm7 | Bb7 | Gm7 | Cm7 | Cb Db | Eb");
    for (beat, len, notes) in &chords {
        s.play(*beat, notes[0] - 24.0, len * 0.9, 0.5, Voice::Bass);
        for &k in &notes[1..] { s.play(*beat, k - 12.0, len * 0.9, 0.07, Voice::Keys) }
        for e in 0..(*len as usize) {
            let i = [0, 1, 2, 3][e % 4] % notes.len();
            s.play(beat + e as f64 + 0.5, notes[i], 1.0, 0.09, Voice::Celesta);
        }
    }
    s.render(16, 3000.0)
}

/// Drizzle: Puddle's slow keys with a harp turning gently over them, round bass and a soft kick. No tune.
fn drizzle() -> Vec<f32> {
    let mut s = Song::new(80.0, 0.6);
    let chords = s.chords("Fmaj9 | Em7 | Dm9 | Cmaj7 | Bbmaj7 | Am7 | Gm9 | C7 | Fmaj9 | Em7 | Dm9 | Db6 | Cmaj7 | Am7 | Gm9 | C7");
    for (beat, _, notes) in &chords {
        for &k in &notes[1..] { s.play(*beat, k - 12.0, 3.6, 0.12, Voice::Keys) }
        for (q, i) in [(0.5, 1), (1.5, 2), (2.5, 3), (3.5, 2)] { s.play(beat + q, notes[i % notes.len()], 1.0, 0.13, Voice::Harp) }
        s.play(*beat, notes[0] - 24.0, 1.8, 0.65, Voice::Bass);
        s.play(beat + 2.5, notes[0] - 17.0, 1.2, 0.45, Voice::Bass);
        for q in [0.0, 2.5] { s.play(beat + q, 0.0, 0.3, 0.28, Voice::Kick) }
        for q in [1.0, 3.0] { s.play(beat + q, 0.0, 0.12, 0.08, Voice::Hat) }
    }
    s.render(16, 3000.0)
}

/// Velvet: electric piano on the bossa off-beats over the bossa bass and shaker, in E-flat. No tune.
fn velvet() -> Vec<f32> {
    let mut s = Song::new(116.0, 0.5);
    let chords = s.chords("Ebmaj9 | Abmaj9 | Gm7 | Cm9 | Fm9 | Bb7 | Ebmaj9 | Bb7 | Abmaj9 | Abm6 | Gm7 | C7 | Fm9 | Bb7 | Ebmaj9 | Bb7");
    for (bar, (beat, _, notes)) in chords.iter().enumerate() {
        let slots: &[f64] = if bar % 2 == 0 { &[0.0, 1.5, 3.0] } else { &[1.0, 2.5] };
        for &off in slots { for &k in &notes[1..] { s.play(beat + off, k - 12.0, 0.9, 0.12, Voice::Keys) } }
        bossa_bass(&mut s, *beat, notes[0]);
        shaker(&mut s, *beat, 8);
    }
    s.render(16, 3400.0)
}

/// Crystal theme: slow celesta and music-box arpeggios over a glassy pad.
fn crystal() -> Vec<f32> {
    let mut s = Song::new(76.0, 0.5);
    let chords = s.chords("Fmaj7 | Dm7 | Bbmaj7 | Csus4 C | Fmaj7 | Am7 | Bbmaj7 | Csus4 C | Dm7 | Bbmaj7 | Gm7 | C7 | Am7 | Dm7 | Db Eb | F");
    for (beat, len, notes) in &chords {
        for &k in &notes[1..] { s.play(*beat, k, *len, 0.05, Voice::Warm) }
        s.play(*beat, notes[0] - 24.0, len * 0.9, 0.45, Voice::Bass);
        for e in 0..(len * 2.0) as usize {
            let i = [0, 2, 1, 3][e % 4] % notes.len();
            s.play(beat + e as f64 * 0.5, notes[i] + 12.0, 1.0, 0.14, if e % 4 == 0 { Voice::Bell } else { Voice::Celesta });
        }
    }
    s.render(16, 3800.0)
}

/// Bubblegum theme: a bouncing bass, popping off-beat chords and a light kick and hat. No tune.
fn bubblegum() -> Vec<f32> {
    let mut s = Song::new(120.0, 0.5);
    let chords = s.chords("C | Am | F | G | C | Em | F | G | Am | F | C | G | F | G | C | G");
    for (beat, _, notes) in &chords {
        for (q, k) in [notes[0] - 24.0, notes[0] - 12.0, notes[2] - 24.0, notes[0] - 12.0].into_iter().enumerate() {
            s.play(beat + q as f64, k, 0.6, 0.6, Voice::Bass)
        }
        for q in 0..4 { for &k in notes.iter() { s.play(beat + q as f64 + 0.5, k + 12.0, 0.12, 0.08, Voice::Triangle) } }
        for q in [0.0, 2.0] { s.play(beat + q, 0.0, 0.3, 0.3, Voice::Kick) }
        for q in 0..8 { s.play(beat + q as f64 * 0.5, 0.0, 0.05, 0.05, Voice::Hat) }
    }
    s.render(16, 3500.0)
}

/// Island theme: guitar on the off-beats, a steel drum on the downbeats, kick, bass and shaker. No tune.
fn island() -> Vec<f32> {
    let mut s = Song::new(104.0, 0.58);
    let chords = s.chords("D | Bm7 | Em7 | A7 | D | F#m7 | G | A7 | Bm7 | G | Em7 | A7 | D | Bm7 | Em7 A7 | D");
    for (beat, len, notes) in &chords {
        let len = *len as usize;
        for q in 0..len { for &k in &notes[1..] { s.play(beat + q as f64 + 0.5, k, 0.3, 0.12, Voice::Pluck) } }
        s.play(*beat, notes[1] + 12.0, 1.0, 0.16, Voice::SteelPan);
        if len == 4 { s.play(beat + 2.0, notes[2] + 12.0, 1.0, 0.13, Voice::SteelPan) }
        s.play(*beat, notes[0] - 24.0, 1.4, 0.6, Voice::Bass);
        if len == 4 { s.play(beat + 2.0, notes[0] - 17.0, 1.4, 0.5, Voice::Bass) }
        s.play(*beat, 0.0, 0.3, 0.25, Voice::Kick);
        shaker(&mut s, *beat, len * 2);
    }
    s.render(16, 3500.0)
}

/// Lounge-keys theme: long electric-piano chords with a vibraphone dropping chord tones in, walking bass and brushes.
fn lounge_keys() -> Vec<f32> {
    let mut s = Song::new(88.0, 0.62);
    let chords = s.chords("Bbmaj9 | Gm9 | Cm9 | F7 | Dm7 | G7 | Cm9 | F7 | Ebmaj9 | Ebm6 | Dm7 | G7 | Cm9 | F7 | Bbmaj9 | F7");
    for (bar, (beat, _, notes)) in chords.iter().enumerate() {
        for &k in &notes[1..] { s.play(*beat, k - 12.0, 3.6, 0.15, Voice::Keys) }
        s.play(beat + 1.5 + (bar % 2) as f64, notes[notes.len() - 1], 1.5, 0.1, Voice::Vibe);
        let r = notes[0] - 24.0;
        let third = r + notes[1] - notes[0];
        for (q, k) in [r, third, r + 7.0, r + 9.0].into_iter().enumerate() { s.play(beat + q as f64, k, 0.9, 0.55, Voice::Bass) }
        for q in [1.0, 3.0] { s.play(beat + q, 0.0, 0.12, 0.09, Voice::Hat) }
    }
    s.render(16, 3200.0)
}
