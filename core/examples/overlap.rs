//! How much of an instance Modrinth hosts byte-for-byte: `cargo run --example overlap -- <game dir>`
fn main() {
    let game = std::path::PathBuf::from(std::env::args().nth(1).expect("game dir"));
    let c = obelisk_core::modrinth::Client::new().unwrap();
    let cache = std::env::temp_dir().join("obelisk-overlap-cache");
    let found = c.installed(&game, &cache).unwrap();
    for kind in obelisk_core::KINDS {
        let total = obelisk_core::list_content(&game, kind).len();
        let on_mr = found.iter().filter(|i| i.kind == kind).count();
        println!("{kind}: {on_mr}/{total} exact files on Modrinth");
    }
}
