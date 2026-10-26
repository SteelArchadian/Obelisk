//! Read-only: sort a profile's mods into client / server / both and print why.
//! cargo run --release --example sides -- <instance id>

use obelisk_core as prism;
use std::collections::HashMap;

fn main() {
    let id = std::env::args().nth(1).expect("instance id");
    let game = prism::game_dir(&prism::data_dir(), &id).expect("no such instance");
    let t = std::time::Instant::now();
    let mods = prism::sides::classify(&game, &HashMap::new());
    println!("{} mods in {:.1}s: {:?}", mods.len(), t.elapsed().as_secs_f32(), prism::sides::counts(&mods));
    for m in &mods {
        if m.side != prism::sides::Side::Both {
            println!("{:?}/{:?}  {}  [{}]  {}", m.side, m.confidence, m.name, m.file_name, m.reasons.join(" | "));
        }
    }
    println!("--- kept on the server only because something needs it:");
    for m in mods.iter().filter(|m| m.reasons.first().is_some_and(|r| r.starts_with("Kept on the server"))) {
        println!("{}  <- {}", m.name, m.needed_by.join(", "));
    }
}
