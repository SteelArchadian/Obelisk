fn main() {
    // Build stamp for one-click updates (selfupdate.rs). Re-stamped whenever the app's sources change.
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    println!("cargo:rustc-env=OBELISK_BUILD={secs}");
    // Whether releases are code-signed (tauri.conf.json names a certificate or sign command): until they are,
    // Obelisk never installs an update without being asked (selfupdate.rs).
    let conf = std::fs::read_to_string("tauri.conf.json").unwrap_or_default();
    let signed = ["\"certificateThumbprint\"", "\"signCommand\""].iter().any(|k| conf.contains(k));
    println!("cargo:rustc-env=OBELISK_SIGNED={}", if signed { "1" } else { "" });
    for p in ["src", "tauri.conf.json", "Cargo.toml", "../src", "../build", "../../core/src"] {
        println!("cargo:rerun-if-changed={p}");
    }
    tauri_build::build()
}
