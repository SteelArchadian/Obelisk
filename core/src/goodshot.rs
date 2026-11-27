//! Good shot: a screenshot only counts when it shows something. Before the grab the world must be joined, no loading
//! screen up and the chunks around the player drawn; after it the picture is judged and a blank, all-black, all-sky
//! or near-uniform frame is rejected (the caller retries). Without this, test runs kept saving pictures of nothing.

use serde_json::{json, Value};
use std::path::Path;

/// Thresholds. A picture fails when it is nearly one flat colour, nearly all black, nearly all sky, or (with a
/// scene check) less of it than `min_fill` is neither sky nor black.
const FLAT_STDDEV: f64 = 6.0;
const MAX_BLACK: f64 = 0.92;
const MAX_SKY: f64 = 0.88;
const DEFAULT_MIN_FILL: f64 = 0.06;

/// What a frame shows, judged from RGBA pixels.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub ok: bool,
    pub why: String,
    pub stddev: f64,
    pub black: f64,
    pub sky: f64,
    pub fill: f64,
}

impl Verdict {
    pub fn json(&self) -> Value {
        let r = |x: f64| (x * 1000.0).round() / 1000.0;
        json!({ "ok": self.ok, "why": self.why, "stddev": r(self.stddev), "black": r(self.black), "sky": r(self.sky), "fill": r(self.fill) })
    }
}

/// Sky: bright-ish and blue-dominant (day sky), or the pale grey-white of fog/overcast at the top edge of the world.
fn is_sky(r: f64, g: f64, b: f64) -> bool {
    (b > r + 18.0 && b >= g - 4.0 && b > 90.0) || (r > 200.0 && g > 200.0 && b > 200.0 && (r - b).abs() < 12.0)
}

/// Judge RGBA (or RGB with `ch` = 3) pixels. Samples a grid of about 40k pixels, so a 4K frame costs the same as a small one.
pub fn judge(px: &[u8], w: u32, h: u32, ch: usize, min_fill: Option<f64>) -> Verdict {
    let total = (w as usize) * (h as usize);
    if total == 0 || px.len() < total * ch {
        return Verdict { ok: false, why: "empty picture".into(), stddev: 0.0, black: 1.0, sky: 0.0, fill: 0.0 };
    }
    let step = ((total as f64 / 40_000.0).sqrt().floor() as usize).max(1);
    let (mut n, mut sum, mut sq, mut black, mut sky) = (0f64, 0f64, 0f64, 0f64, 0f64);
    for y in (0..h as usize).step_by(step) {
        for x in (0..w as usize).step_by(step) {
            let i = (y * w as usize + x) * ch;
            let (r, g, b) = (px[i] as f64, px[i + 1] as f64, px[i + 2] as f64);
            let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            n += 1.0;
            sum += l;
            sq += l * l;
            if l < 14.0 { black += 1.0 } else if is_sky(r, g, b) { sky += 1.0 }
        }
    }
    let mean = sum / n;
    let stddev = (sq / n - mean * mean).max(0.0).sqrt();
    let (black, sky) = (black / n, sky / n);
    let fill = (1.0 - black - sky).max(0.0);
    let need = min_fill.unwrap_or(DEFAULT_MIN_FILL);
    let why = if stddev < FLAT_STDDEV { format!("blank: nearly one flat colour (contrast {stddev:.1})") }
        else if black > MAX_BLACK { format!("all black ({:.0}% black)", black * 100.0) }
        else if sky > MAX_SKY { format!("all sky ({:.0}% sky)", sky * 100.0) }
        else if fill < need { format!("too little scene: {:.0}% is neither sky nor black, need {:.0}%", fill * 100.0, need * 100.0) }
        else { String::new() };
    Verdict { ok: why.is_empty(), why, stddev, black, sky, fill }
}

/// Read a PNG and judge it. The game writes screenshots a moment after the grab, so a missing or half-written file is
/// retried for up to `wait_s` seconds.
pub fn judge_file(path: &Path, min_fill: Option<f64>, wait_s: f64) -> Result<Verdict, String> {
    let t0 = std::time::Instant::now();
    loop {
        match decode(path) {
            Ok((w, h, ch, px)) => return Ok(judge(&px, w, h, ch, min_fill)),
            Err(e) if t0.elapsed().as_secs_f64() >= wait_s => return Err(format!("can't read the screenshot {}: {e}", path.display())),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(150)),
        }
    }
}

fn decode(path: &Path) -> Result<(u32, u32, usize, Vec<u8>), String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut d = png::Decoder::new(std::io::BufReader::new(f));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = d.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; r.output_buffer_size().ok_or("picture too large")?];
    let info = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let ch = match info.color_type { png::ColorType::Rgba => 4, png::ColorType::Rgb => 3, png::ColorType::GrayscaleAlpha => 2, _ => 1 };
    if ch < 3 {
        // Grey pictures: spread to RGB so one judge fits all.
        let g: Vec<u8> = buf[..info.buffer_size()].chunks_exact(ch).flat_map(|p| [p[0], p[0], p[0]]).collect();
        return Ok((info.width, info.height, 3, g));
    }
    buf.truncate(info.buffer_size());
    Ok((info.width, info.height, ch, buf))
}

/// Is the game ready for a picture? (world joined, no screen in front, every nearby section drawn). None = ready,
/// Some(why) = not yet.
pub fn not_ready(q: &Value) -> Option<String> {
    if q["busy"] == true { return Some("the game is busy".into()) }
    if q["inWorld"] != true { return Some("not in a world yet".into()) }
    if let Some(s) = q["screen"].as_str() { return Some(format!("a screen is open ({s})")) }
    if q["renderReady"] == false { return Some(format!("chunks still drawing ({} loaded)", q["chunksLoaded"])) }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: u32, h: u32, c: [u8; 3]) -> Vec<u8> { (0..w * h).flat_map(|_| [c[0], c[1], c[2], 255]).collect() }

    /// A frame with sky on top and varied ground below, like a normal outdoor shot.
    fn scene(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::new();
        for y in 0..h {
            for x in 0..w {
                if y < h / 3 { v.extend([120, 165, 255, 255]) }
                else { let k = ((x * 7 + y * 13) % 60) as u8; v.extend([60 + k, 110 + k / 2, 40 + k / 3, 255]) }
            }
        }
        v
    }

    /// Junk frames the old shot step saved without a word: each must now be rejected.
    #[test]
    fn rejects_junk() {
        let (w, h) = (320, 180);
        assert!(!judge(&flat(w, h, [0, 0, 0]), w, h, 4, None).ok, "all black");
        assert!(!judge(&flat(w, h, [120, 165, 255]), w, h, 4, None).ok, "flat sky");
        // Loading screen: dark dirt-brown, nearly flat.
        assert!(!judge(&flat(w, h, [42, 30, 22]), w, h, 4, None).ok, "loading screen");
        // Sky with a gradient (has contrast, still nothing in it).
        let grad: Vec<u8> = (0..h).flat_map(|y| (0..w).flat_map(move |_| [90 + (y / 4) as u8, 140 + (y / 6) as u8, 255, 255])).collect();
        let v = judge(&grad, w, h, 4, None);
        assert!(!v.ok && v.why.contains("sky"), "sky gradient: {v:?}");
    }

    #[test]
    fn keeps_scenes() {
        let (w, h) = (320, 180);
        let v = judge(&scene(w, h), w, h, 4, None);
        assert!(v.ok, "{v:?}");
        // A stricter scene check can still fail it.
        assert!(!judge(&scene(w, h), w, h, 4, Some(0.9)).ok);
    }

    #[test]
    fn readiness() {
        assert!(not_ready(&json!({ "inWorld": false })).is_some());
        assert!(not_ready(&json!({ "inWorld": true, "screen": "LevelLoadingScreen" })).is_some());
        assert!(not_ready(&json!({ "inWorld": true, "screen": null, "renderReady": false, "chunksLoaded": 40 })).is_some());
        assert!(not_ready(&json!({ "inWorld": true, "screen": null, "renderReady": true })).is_none());
    }
}
