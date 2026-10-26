//! Two Obelisk peers in one process, over the real network: Alex shares, Sam joins with the invite
//! code, then changes flow both ways. Uses temp folders only.
//! cargo run --release --example share_demo

use obelisk_core::{share, share_net::Net};
use std::fs;
use std::time::Instant;

#[tokio::main]
async fn main() {
    let tmp = tempfile::tempdir().unwrap();
    let (a_inst, b_inst) = (tmp.path().join("alex"), tmp.path().join("sam"));
    let a_game = a_inst.join("Pack/minecraft");
    fs::create_dir_all(a_game.join("mods")).unwrap();
    fs::create_dir_all(a_game.join("config")).unwrap();
    fs::create_dir_all(&b_inst).unwrap();
    fs::write(a_inst.join("Pack/instance.cfg"), "InstanceType=OneSix\nname=Pack\n").unwrap();
    fs::write(a_inst.join("Pack/mmc-pack.json"), r#"{"components":[{"uid":"net.minecraft","version":"1.21.1"}],"formatVersion":1}"#).unwrap();
    fs::write(a_game.join("mods/big.jar"), vec![7u8; 5 << 20]).unwrap();
    fs::write(a_game.join("config/a.toml"), "x=1").unwrap();

    let key = |b: u8| [b; 32];
    let alex = Net::start(key(1), a_inst.clone(), "Alex".into()).await.unwrap();
    let sam = Net::start(key(2), b_inst.clone(), "Sam".into()).await.unwrap();

    let t = Instant::now();
    let code = alex.share(&a_game, "Pack").await.unwrap();
    println!("invite ({} chars) ready in {:.1}s", code.len(), t.elapsed().as_secs_f32());

    let t = Instant::now();
    let id = sam.join(&code).await.unwrap();
    let b_game = b_inst.join(&id).join("minecraft");
    println!("Sam joined as '{id}' in {:.1}s; big.jar copied: {}", t.elapsed().as_secs_f32(), fs::metadata(b_game.join("mods/big.jar")).map(|m| m.len()).unwrap_or(0) == 5 << 20);

    // Alex changes a config and publishes; Sam pulls and sees it as incoming.
    fs::write(a_game.join("config/a.toml"), "x=2").unwrap();
    alex.publish(&a_game, "Tweak").unwrap().unwrap();
    let t = Instant::now();
    let pulled = sam.sync_all().await;
    let inc = share::incoming(&b_game);
    println!("Sam pulled {pulled:?} in {:.1}s; incoming: {:?}", t.elapsed().as_secs_f32(), inc.iter().map(|c| (&c.author_name, &c.label, c.changes.len())).collect::<Vec<_>>());
    share::apply(&b_game, &[]).unwrap();
    println!("after apply Sam's config: {}", fs::read_to_string(b_game.join("config/a.toml")).unwrap());

    // And back: Sam adds a mod, Alex pulls.
    fs::write(b_game.join("mods/jei.jar"), "jei").unwrap();
    sam.publish(&b_game, "").unwrap().unwrap();
    let pulled = alex.sync_all().await;
    println!("Alex pulled {pulled:?}; members now: {:?}", share::get(&a_game).unwrap().members.iter().map(|m| &m.name).collect::<Vec<_>>());
}
