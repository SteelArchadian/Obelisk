//! Lightning (the user's profiler mod) integration: reads its capture results so Obelisk can
//! show a history of runs and call out the mods that cost the most.
//! Format: `<game>/lightning/captures/lightning-<id>.results.json` (schema_version 2; fields are
//! only ever added, never renamed) next to `lightning-<id>.html`.

use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    pub label: String,
    pub started_ms: u64,
    /// Summary numbers for the whole capture plus per-mod costs, findings and "contenders".
    pub whole: Value,
    pub findings: Value,
    pub contenders: Value,
    pub phases: Vec<String>,
    /// The capture's HTML report, if it exists (relative to the game folder).
    pub html: String,
}

/// Is Lightning installed in this profile (enabled or not)?
pub fn installed(game: &Path) -> bool {
    fs::read_dir(game.join("mods")).into_iter().flatten().flatten().any(|e| {
        let n = e.file_name().to_string_lossy().to_lowercase();
        n.starts_with("lightning") && (n.ends_with(".jar") || n.ends_with(".jar.disabled"))
    })
}

/// Lightning runs in this profile, newest first.
pub fn runs(game: &Path, limit: usize) -> Vec<Run> {
    let dir = game.join("lightning").join("captures");
    let Ok(entries) = fs::read_dir(&dir) else { return vec![] };
    let mut files: Vec<_> = entries.flatten().map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".results.json")).collect();
    files.sort_by_key(|p| std::cmp::Reverse(fs::metadata(p).and_then(|m| m.modified()).ok()));
    files.into_iter().take(limit).filter_map(|p| {
        let v: Value = serde_json::from_slice(&fs::read(&p).ok()?).ok()?;
        let id = v["id"].as_str().map(str::to_string).unwrap_or_else(|| {
            p.file_name().unwrap_or_default().to_string_lossy().trim_start_matches("lightning-").trim_end_matches(".results.json").to_string()
        });
        let html_name = format!("lightning-{id}.html");
        let html = if dir.join(&html_name).is_file() { format!("lightning/captures/{html_name}") } else { String::new() };
        let whole = &v["whole"];
        let pick = |k: &str| whole.get(k).cloned().unwrap_or(Value::Null);
        Some(Run {
            label: v["label"].as_str().unwrap_or("").to_string(),
            started_ms: v["startedAt"].as_u64().unwrap_or(0),
            whole: json!({
                "seconds": pick("seconds"), "fps": pick("fps"), "frameP50": pick("frameP50"), "frameP99": pick("frameP99"),
                "frameMax": pick("frameMax"), "low1PctFps": pick("low1PctFps"), "gpuBoundPct": pick("gpuBoundPct"),
                "tickP50": pick("tickP50"), "tickMax": pick("tickMax"), "ticksOver50ms": pick("ticksOver50ms"),
                "heapMaxMB": pick("heapMaxMB"), "liveSetMBmax": pick("liveSetMBmax"), "gcPauseMs": pick("gcPauseMs"),
                "allocMBps": pick("allocMBps"), "topModsCpu": pick("topModsCpu"), "allocByMod": pick("allocByMod"), "spikes": pick("spikes"),
            }),
            findings: v["findings"].clone(),
            contenders: v["contenders"].clone(),
            phases: v["phaseOrder"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect(),
            html,
            id,
        })
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_runs_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let g = tmp.path();
        let dir = g.join("lightning/captures");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(g.join("mods")).unwrap();
        assert!(!installed(g));
        fs::write(g.join("mods/lightning-0.3.0.jar"), "").unwrap();
        assert!(installed(g));
        fs::write(dir.join("lightning-a.results.json"), r#"{"id":"a","label":"first","startedAt":1,"whole":{"fps":100,"topModsCpu":[{"mod":"x","sharePct":40}]},"findings":[],"contenders":[]}"#).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(dir.join("lightning-b.results.json"), r#"{"id":"b","whole":{"fps":50}}"#).unwrap();
        fs::write(dir.join("lightning-b.html"), "").unwrap();
        let r = runs(g, 10);
        assert_eq!(r.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(r[0].html, "lightning/captures/lightning-b.html");
        assert_eq!(r[1].whole["topModsCpu"][0]["mod"], "x");
    }
}
