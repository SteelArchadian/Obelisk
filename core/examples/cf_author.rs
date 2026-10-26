//! Live check: every Minecraft project by the author of a CurseForge project. Key from Credential Manager.
//! cargo run --example cf_author -- <project id>
use obelisk_core::curseforge::Client;

fn main() {
    let id: u64 = std::env::args().nth(1).expect("project id").parse().unwrap();
    let key = keyring::Entry::new("Obelisk", "curseforge-api-key").unwrap().get_password().expect("no key saved");
    let c = Client::new(&key).unwrap();
    let p = c.project(id).unwrap();
    let a = &p["authors"][0];
    let list = c.author_projects(a["id"].as_u64().unwrap()).unwrap();
    let mut versions: Vec<String> = list.iter().flat_map(|m| m["latestFilesIndexes"].as_array().cloned().unwrap_or_default())
        .filter_map(|i| i["gameVersion"].as_str().map(str::to_string)).collect();
    versions.sort(); versions.dedup();
    println!("{}: {} projects (games: {:?}); classes {:?}; versions {}..{}", a["name"], list.len(),
        list.iter().map(|m| m["gameId"].as_u64().unwrap_or(0)).collect::<std::collections::BTreeSet<_>>(),
        list.iter().map(|m| m["classId"].as_u64().unwrap_or(0)).collect::<std::collections::BTreeSet<_>>(),
        versions.first().cloned().unwrap_or_default(), versions.last().cloned().unwrap_or_default());
    for m in list.iter().take(5) { println!("  {}", m["name"]); }
}
