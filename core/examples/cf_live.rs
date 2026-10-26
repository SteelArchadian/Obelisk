//! Live CurseForge checks using the key saved by Obelisk (never printed): `cargo run --example cf_live`
use obelisk_core::curseforge::{fingerprint, Client, SearchQuery};
fn main() {
    let key = keyring::Entry::new("Obelisk", "curseforge-api-key").unwrap().get_password().expect("no key saved");
    let c = Client::new(&key).unwrap();
    // 1. Our fingerprint matches what CurseForge recorded for a real installed file.
    // A CurseForge app instance folder: `cargo run --example cf_live -- <instance folder>`.
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("give a CurseForge instance folder"));
    let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("minecraftinstance.json")).unwrap()).unwrap();
    let a = manifest["installedAddons"].as_array().unwrap().iter()
        .find(|a| a["categoryClassID"] == 6 && a["isEnabled"] == true).unwrap();
    let path = dir.join("mods").join(a["fileNameOnDisk"].as_str().unwrap());
    let fp = fingerprint(&std::fs::read(&path).unwrap());
    println!("fingerprint match: {} ({})", fp as u64 == a["installedFile"]["packageFingerprint"].as_u64().unwrap(), a["name"]);
    // 2. Search.
    let q = SearchQuery { query: "jade".into(), project_type: "mod".into(), loader: "neoforge".into(), game_version: "1.21.1".into(), limit: 5, ..Default::default() };
    let r = c.search(&q).unwrap();
    println!("search hits: {}", r["data"].as_array().map(|a| a.len()).unwrap_or(0));
    // 3. Fingerprint lookup finds the project.
    let m = c.match_fingerprints(&[fp]).unwrap();
    println!("fingerprint lookup -> mod id {:?} (expected {})", m.first().map(|x| x.1), a["addonID"]);
    // 4. Files for that mod on this game version.
    let files = c.files(a["addonID"].as_u64().unwrap(), "1.21.1", "neoforge").unwrap();
    println!("files for 1.21.1 neoforge: {}", files.len());
}
