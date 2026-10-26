//! Live update check on the Prism Foundation copy (read-only): `cargo run --release --example updates`
fn main() {
    let key = keyring::Entry::new("Obelisk", "curseforge-api-key").ok().and_then(|e| e.get_password().ok());
    let cf = key.as_deref().map(|k| obelisk_core::curseforge::Client::new(k).unwrap());
    let mr = obelisk_core::modrinth::Client::new().unwrap();
    let game = obelisk_core::game_dir(&obelisk_core::data_dir(), "Foundation-prism").unwrap();
    let t = std::time::Instant::now();
    let infos = obelisk_core::updates::check(&game, &std::env::temp_dir().join("obelisk-upd-cache"), &mr, cf.as_ref(), "NeoForge", "1.21.1").unwrap();
    let count = |p: &str| infos.iter().filter(|i| i.provider == p).count();
    println!("{} files in {:.1}s: modrinth {}, curseforge {}, unknown {}", infos.len(), t.elapsed().as_secs_f32(), count("modrinth"), count("curseforge"), count(""));
    let ups: Vec<_> = infos.iter().filter(|i| i.update.is_some()).collect();
    println!("updates: {} ({} blocked)", ups.len(), ups.iter().filter(|i| i.update.as_ref().unwrap().blocked).count());
    for i in ups.iter().take(4) { let u = i.update.as_ref().unwrap(); println!("  {} -> {} [{}]", i.file_name, u.file_name, i.provider); }
    println!("with downloads: {}, with env: {}", infos.iter().filter(|i| i.downloads > 0).count(), infos.iter().filter(|i| !i.env.is_empty()).count());
}
