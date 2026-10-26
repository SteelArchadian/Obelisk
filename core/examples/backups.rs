//! Exercise backups on a game folder (use a copy, it writes .obelisk/history into it).
//! cargo run --release --example backups -- <game dir>

use obelisk_core::history;
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let game = PathBuf::from(std::env::args().nth(1).expect("game dir"));
    let t = Instant::now();
    let b = history::create(&game, "manual", "Start", false).unwrap().unwrap();
    println!("first backup: {} files, {} MB of pack, {:.1}s, store uses {} MB", b.files, b.bytes >> 20, t.elapsed().as_secs_f32(), history::disk_use(&game) >> 20);

    let t = Instant::now();
    println!("no-change check: {:?} in {:.2}s", history::pending(&game).unwrap().len(), t.elapsed().as_secs_f32());

    // A typical session: remove a mod, edit a config.
    let mods = game.join("mods");
    let jar = std::fs::read_dir(&mods).unwrap().flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "jar")).unwrap();
    std::fs::remove_file(&jar).unwrap();
    let cfg = std::fs::read_dir(game.join("config")).unwrap().flatten().map(|e| e.path()).find(|p| p.is_file()).unwrap();
    let mut text = std::fs::read_to_string(&cfg).unwrap_or_default();
    text.push_str("\n# edited\n");
    std::fs::write(&cfg, text).unwrap();

    let t = Instant::now();
    let b2 = history::create(&game, "auto", "", true).unwrap().unwrap();
    println!("second backup in {:.2}s, changes: {:?}", t.elapsed().as_secs_f32(), b2.changes.iter().map(|c| format!("{} {}", c.kind, c.label)).collect::<Vec<_>>());
    println!("store now uses {} MB", history::disk_use(&game) >> 20);

    let t = Instant::now();
    history::revert_change(&game, &b2.id, &b2.changes.iter().find(|c| c.kind == "removed").unwrap().path).unwrap();
    println!("brought the mod back in {:.2}s: exists={}", t.elapsed().as_secs_f32(), jar.is_file());
    let t = Instant::now();
    history::restore(&game, &b.id).unwrap();
    println!("full restore in {:.2}s, pending after: {}", t.elapsed().as_secs_f32(), history::pending(&game).unwrap().len());
}
