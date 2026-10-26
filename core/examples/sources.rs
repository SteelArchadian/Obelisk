//! List what Sync would see (read-only): `cargo run --example sources`
fn main() {
    for l in obelisk_core::migrate::detect_launchers() {
        println!("{:<12} found={} supported={}", l.name, l.found, l.supported);
    }
    let reg = obelisk_core::migrate::curseforge_registry();
    for p in obelisk_core::migrate::curseforge_profiles(&reg, &[], &obelisk_core::data_dir()) {
        println!("{} | {} {} {} | {} h | synced as {:?}", p.name, p.minecraft, p.loader_uid, p.loader_version, p.seconds_played / 3600, p.synced_as);
    }
}
