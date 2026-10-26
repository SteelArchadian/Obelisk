//! Live check: CurseForge search with several loaders at once. Key from Credential Manager.
//! cargo run --example cf_loaders -- <query> <loader> [<loader>...]
use obelisk_core::curseforge::{Client, SearchQuery};

fn main() {
    let mut a = std::env::args().skip(1);
    let query = a.next().expect("query");
    let loaders: Vec<String> = a.collect();
    let key = keyring::Entry::new("Obelisk", "curseforge-api-key").unwrap().get_password().expect("no key saved");
    let q = SearchQuery { query, project_type: "mod".into(), loaders, game_version: "1.21.1".into(), limit: 8, ..Default::default() };
    let r = Client::new(&key).unwrap().search(&q).unwrap();
    println!("total {}", r["pagination"]["totalCount"]);
    for m in r["data"].as_array().unwrap() {
        let types: std::collections::BTreeSet<u64> = m["latestFilesIndexes"].as_array().unwrap().iter()
            .filter(|i| i["gameVersion"] == "1.21.1").filter_map(|i| i["modLoader"].as_u64()).collect();
        println!("  {} {:?}", m["name"], types);
    }
}
