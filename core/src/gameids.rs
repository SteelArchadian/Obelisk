//! The mob, block and item ids a pack has, with their English names, for config fields that ask for them.
//! Read from each jar's assets/<ns>/lang/en_us.json (Minecraft's own client jar included): every entity, block
//! and item a mod adds has a name there, so the keys double as a registry list.

use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Serialize, Default, Clone)]
pub struct GameIds {
    /// [id, name] pairs, sorted by id.
    pub entity: Vec<(String, String)>,
    pub block: Vec<(String, String)>,
    pub item: Vec<(String, String)>,
}

static CACHE: Mutex<Option<HashMap<PathBuf, (u64, Arc<GameIds>)>>> = Mutex::new(None);

/// Minecraft's client jar for this version, from Prism's library store.
pub fn client_jar(data: &Path, minecraft: &str) -> PathBuf {
    data.join("libraries/com/mojang/minecraft").join(minecraft).join(format!("minecraft-{minecraft}-client.jar"))
}

/// The pack's ids, cached until its mods folder changes.
pub fn for_pack(game: &Path, client: &Path) -> Arc<GameIds> {
    let jars = jars(game, client);
    let stamp = jars.iter().fold(0u64, |h, p| {
        let m = std::fs::metadata(p).ok();
        let t = m.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        h.wrapping_mul(31).wrapping_add(t ^ m.map_or(0, |m| m.len())).wrapping_add(p.as_os_str().len() as u64)
    });
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = g.get_or_insert_with(HashMap::new);
    if let Some((s, ids)) = cache.get(game) {
        if *s == stamp {
            return ids.clone();
        }
    }
    let ids = Arc::new(scan(&jars));
    cache.insert(game.to_path_buf(), (stamp, ids.clone()));
    ids
}

fn jars(game: &Path, client: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(game.join("mods")).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("jar"))).collect();
    out.sort();
    if client.is_file() {
        out.insert(0, client.to_path_buf());
    }
    out
}

fn scan(jars: &[PathBuf]) -> GameIds {
    let mut kinds: [BTreeMap<String, String>; 3] = Default::default();
    for jar in jars {
        let Ok(mut z) = std::fs::File::open(jar).map_err(|_| ()).and_then(|f| zip::ZipArchive::new(f).map_err(|_| ())) else { continue };
        let langs: Vec<String> = z.file_names().filter(|n| n.starts_with("assets/") && n.ends_with("/lang/en_us.json")).map(String::from).collect();
        for name in langs {
            let mut text = String::new();
            if z.by_name(&name).ok().and_then(|mut f| f.read_to_string(&mut text).ok()).is_none() {
                continue;
            }
            add(&mut kinds, text.trim_start_matches('\u{feff}'));
        }
    }
    let [entity, block, item] = kinds.map(|m| m.into_iter().collect());
    GameIds { entity, block, item }
}

/// Keys like "entity.minecraft.donkey" -> minecraft:donkey. Longer keys ("block.minecraft.banner.base") are
/// sub-strings, not things in the game.
fn add(kinds: &mut [BTreeMap<String, String>; 3], text: &str) {
    let Ok(map) = serde_json::from_str::<BTreeMap<String, serde_json::Value>>(text) else { return };
    for (k, v) in map {
        let mut parts = k.split('.');
        let (Some(kind), Some(ns), Some(path), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else { continue };
        let i = match kind { "entity" => 0, "block" => 1, "item" => 2, _ => continue };
        let ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-/".contains(&b));
        if !ok(ns) || !ok(path) {
            continue;
        }
        let name = v.as_str().unwrap_or_default().to_string();
        kinds[i].entry(format!("{ns}:{path}")).or_insert(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lang_keys() {
        let mut k: [BTreeMap<String, String>; 3] = Default::default();
        add(&mut k, r#"{"entity.minecraft.donkey":"Donkey","block.minecraft.banner.base":"x","block.astikorcarts.cart":"Cart","item.mymod.Bad":"no","gui.x.y":"z"}"#);
        assert_eq!(k[0].get("minecraft:donkey").map(String::as_str), Some("Donkey"));
        assert!(k[1].contains_key("astikorcarts:cart") && k[1].len() == 1);
        assert!(k[2].is_empty());
    }
}
