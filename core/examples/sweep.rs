//! Read-only: diagnose every crash report and JVM crash log in the game folders listed on stdin (one per line)
//! and print one JSON line per file. `cargo run --release --example sweep < dirs.txt`
use std::io::BufRead;
fn main() {
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let game = std::path::PathBuf::from(line.trim());
        for r in obelisk_core::crash::list(&game).into_iter().filter(|r| r.kind != "log") {
            let row = match obelisk_core::crash::diagnose(&game, &r.file) {
                Ok(d) => serde_json::json!({
                    "game": game, "file": r.file, "kind": r.kind, "headline": d.headline,
                    "error": d.error.chars().take(200).collect::<String>(),
                    "findings": d.findings.iter().map(|f| serde_json::json!({"severity": f.severity, "title": f.title, "detail": f.detail, "fix": f.fix})).collect::<Vec<_>>(),
                    "suspects": d.suspects.iter().map(|s| serde_json::json!({"mod": s.mod_id, "reason": s.reason, "score": s.score})).collect::<Vec<_>>(),
                }),
                Err(e) => serde_json::json!({"game": game, "file": r.file, "kind": r.kind, "failed": e}),
            };
            println!("{row}");
        }
    }
}
