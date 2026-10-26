//! Print what Obelisk sees in the local Prism data folder: `cargo run --example dump`
fn main() {
    let data = obelisk_core::data_dir();
    for inst in obelisk_core::list_instances(&data) {
        println!("{} [{}] mc {} {} {} played {}h", inst.name, inst.id, inst.minecraft,
                 inst.loader, inst.loader_version, inst.seconds_played / 3600);
        if let Some(game) = obelisk_core::game_dir(&data, &inst.id) {
            for kind in obelisk_core::KINDS {
                let items = obelisk_core::list_content(&game, kind);
                let local = items.iter().filter(|i| i.source == "local").count();
                let authored = items.iter().filter(|i| !i.authors.is_empty()).count();
                println!("  {kind}: {} items, {local} local, {authored} with authors", items.len());
            }
        }
    }
}
