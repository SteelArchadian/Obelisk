//! Dry run: what moving the modpacks folder to the given path would do (nothing is changed).
fn main() {
    let to = std::env::args().nth(1).expect("usage: instdir_plan <folder>");
    let data = obelisk_core::data_dir();
    match obelisk_core::instdir::plan(&data, std::path::Path::new(&to)) {
        Ok(p) => println!("ok: {} -> {}: {} entries, {} files, {} MB", p.from.display(), p.to.display(), p.names.len(), p.files, p.bytes >> 20),
        Err(e) => println!("refused: {e}"),
    }
}
