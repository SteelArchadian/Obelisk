//! Live check: the raw author objects of a CurseForge project. Key from Credential Manager.
//! cargo run --example cf_authors_raw -- <project id>
use obelisk_core::curseforge::Client;

fn main() {
    let id: u64 = std::env::args().nth(1).expect("project id").parse().unwrap();
    let key = keyring::Entry::new("Obelisk", "curseforge-api-key").unwrap().get_password().expect("no key saved");
    let p = Client::new(&key).unwrap().project(id).unwrap();
    println!("{}", serde_json::to_string_pretty(&p["authors"]).unwrap());
}
