fn main() {
    // Build stamp for one-click updates (selfupdate.rs). Re-stamped whenever the app's sources change.
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    println!("cargo:rustc-env=OBELISK_BUILD={secs}");
    for p in ["src", "tauri.conf.json", "Cargo.toml", "../src", "../build", "../../core/src"] {
        println!("cargo:rerun-if-changed={p}");
    }
    tauri_build::build()
}
