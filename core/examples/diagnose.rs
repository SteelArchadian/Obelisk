//! Read-only: explain every crash file in a game folder. `cargo run --example diagnose -- <game dir>`
fn main() {
    let game = std::path::PathBuf::from(std::env::args().nth(1).expect("game dir"));
    for r in obelisk_core::crash::list(&game) {
        let d = obelisk_core::crash::diagnose(&game, &r.file).unwrap();
        println!("== {} [{}]\n  HEADLINE: {}\n  ERROR: {}", r.file, r.kind, d.headline, d.error.chars().take(120).collect::<String>());
        for f in &d.findings { println!("  - [{}] {} | {} | FIX: {} | {}:{}", f.severity, f.title, f.detail.replace('\n', "; "), f.fix, f.action, f.target); }
        for s in &d.suspects { println!("  * {} ({}) score {} - {}", s.mod_id, s.file_name, s.score, s.reason); }
    }
}
