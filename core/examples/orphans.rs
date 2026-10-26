//! Read-only: list config entries no installed mod seems to own. `cargo run --example orphans -- <game dir>`
fn main() {
    let game = std::path::PathBuf::from(std::env::args().nth(1).expect("game dir"));
    let o = obelisk_core::configs::orphans(&game);
    println!("{} orphans", o.len());
    for n in o { println!("  {n}"); }
}
