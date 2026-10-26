//! Build a server pack (and optionally the client pack) for a profile into a folder. Only reads the profile.
//! cargo run --release --example serverpack -- <instance id> <out dir> [--client]

use obelisk_core as prism;
use std::collections::HashMap;
use std::path::PathBuf;

fn main() {
    let mut a = std::env::args().skip(1);
    let id = a.next().expect("instance id");
    let out = PathBuf::from(a.next().expect("out dir"));
    let make_client = a.next().as_deref() == Some("--client");
    let data = prism::data_dir();
    let game = prism::game_dir(&data, &id).expect("no such instance");
    let inst = prism::list_instances(&data).into_iter().find(|i| i.id == id).expect("instance");
    let mods = prism::sides::classify(&game, &HashMap::new());
    let plan = prism::sides::PackPlan {
        out_dir: out,
        name: inst.name.clone(),
        minecraft: inst.minecraft.clone(),
        loader: inst.loader.clone(),
        loader_version: inst.loader_version.clone(),
        max_mem_mb: 6144,
        server_mods: mods.iter().filter(|m| m.side != prism::sides::Side::Client).map(|m| m.file_name.clone()).collect(),
        client_drop: mods.iter().filter(|m| m.side == prism::sides::Side::Server).map(|m| m.file_name.clone()).collect(),
        make_client,
    };
    let mr = prism::modrinth::Client::new().expect("http");
    let t = std::time::Instant::now();
    match prism::sides::build(&prism::instances_dir(&data).join(&id), &game, &plan, &|u| mr.fetch(u)) {
        Ok(b) => println!("built in {:.1}s: {b:#?}", t.elapsed().as_secs_f32()),
        Err(e) => println!("failed: {e}"),
    }
}
