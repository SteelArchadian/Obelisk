//! The same mod installed twice, usually once from Modrinth and once from CurseForge (their files
//! differ, so ids and hashes don't match, but the mod id inside the jar does). Two enabled jars with
//! one mod id stop NeoForge/Forge/Fabric from starting, so installs refuse a second copy and the
//! mods list flags any that slipped in.

use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// The mod ids a jar itself declares (not the libraries bundled inside it, which loaders dedupe).
pub fn own_ids(jar: &Path) -> Vec<String> {
    fs::File::open(jar).map(|f| crate::configs::ids_in(f, 1)).unwrap_or_default()
}

pub fn own_ids_of(bytes: &[u8]) -> Vec<String> {
    crate::configs::ids_in(std::io::Cursor::new(bytes), 1)
}

/// Mod id -> file names, for the enabled jars in a mods folder.
pub fn index(mods: &Path) -> HashMap<String, Vec<String>> {
    let mut by_id: HashMap<String, Vec<String>> = HashMap::new();
    for e in fs::read_dir(mods).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.to_lowercase().ends_with(".jar") { continue }
        for id in own_ids(&e.path()) {
            let files = by_id.entry(id).or_default();
            if !files.contains(&name) { files.push(name.clone()) }
        }
    }
    by_id
}

/// An installed jar (other than `file_name` itself) that already provides one of `ids`.
pub fn clash(index: &HashMap<String, Vec<String>>, ids: &[String], file_name: &str) -> Option<(String, String)> {
    ids.iter().find_map(|id| {
        index.get(id)?.iter().find(|f| f.as_str() != file_name).map(|f| (id.clone(), f.clone()))
    })
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Duplicate {
    pub mod_id: String,
    pub files: Vec<String>,
}

/// Mods present more than once among a profile's enabled jars.
pub fn find(game: &Path) -> Vec<Duplicate> {
    let mut out: Vec<Duplicate> = index(&game.join("mods")).into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(mod_id, mut files)| { files.sort(); Duplicate { mod_id, files } })
        .collect();
    out.sort_by(|a, b| a.mod_id.cmp(&b.mod_id));
    out
}

/// Refuse to start a game with a mod installed twice: NeoForge stops at the loading screen anyway.
pub fn check_before_launch(game: &Path) -> Result<(), String> {
    let found = find(game);
    if found.is_empty() { return Ok(()) }
    let list: Vec<String> = found.iter().map(|d| format!("{} ({})", d.mod_id, d.files.join(", "))).collect();
    Err(format!("Not starting: {} installed twice: {}. Disable one copy of each in the mods list.",
        if found.len() == 1 { "a mod is".to_string() } else { format!("{} mods are", found.len()) }, list.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn jar(dir: &Path, name: &str, id: &str) {
        let mut z = zip::ZipWriter::new(fs::File::create(dir.join(name)).unwrap());
        z.start_file("META-INF/neoforge.mods.toml", zip::write::SimpleFileOptions::default()).unwrap();
        write!(z, "[[mods]]\nmodId=\"{id}\"\n").unwrap();
        z.finish().unwrap();
    }

    #[test]
    fn finds_the_same_mod_from_both_sites() {
        let game = std::env::temp_dir().join(format!("obelisk-dupes-{}", std::process::id()));
        let mods = game.join("mods");
        fs::create_dir_all(&mods).unwrap();
        jar(&mods, "chat_heads-0.15.7-neoforge-1.21.jar", "chat_heads");
        jar(&mods, "chat-heads-0.15.7-neoforge.jar", "chat_heads");
        jar(&mods, "jei-19.jar", "jei");
        jar(&mods, "old.jar.disabled", "jei");
        let d = find(&game);
        assert_eq!(d, vec![Duplicate { mod_id: "chat_heads".into(),
            files: vec!["chat-heads-0.15.7-neoforge.jar".into(), "chat_heads-0.15.7-neoforge-1.21.jar".into()] }]);
        let idx = index(&mods);
        assert_eq!(clash(&idx, &["jei".into()], "jei-20.jar"), Some(("jei".into(), "jei-19.jar".into())));
        assert_eq!(clash(&idx, &["jei".into()], "jei-19.jar"), None);
        let _ = fs::remove_dir_all(&game);
    }
}

/// Every mod id a jar provides, the ones in its bundled jars too (Kotlin for Forge declares its id only inside one).
pub fn provides(jar: &Path) -> Vec<String> {
    fs::File::open(jar).map(|f| crate::configs::ids_in(f, 0)).unwrap_or_default()
}

/// The mod ids a jar requires to load (NeoForge/Forge mods.toml required dependencies and a language provider other
/// than Java's, Fabric "depends"), its bundled jars' too, without the game, the loader and Java themselves.
pub fn needs(jar: &Path) -> Vec<String> {
    let Ok(f) = fs::File::open(jar) else { return vec![] };
    let mut out = needs_in(f, true);
    out.retain(|id| !matches!(id.as_str(), "minecraft" | "neoforge" | "forge" | "fabricloader" | "java" | "javafml" | "lowcodefml"));
    out.sort();
    out.dedup();
    out
}

fn needs_in<R: std::io::Read + std::io::Seek>(reader: R, nested: bool) -> Vec<String> {
    use std::io::Read;
    let Ok(mut zip) = zip::ZipArchive::new(reader) else { return vec![] };
    let mut read = |name: &str| -> Option<String> {
        let mut s = String::new();
        zip.by_name(name).ok()?.read_to_string(&mut s).ok()?;
        Some(s)
    };
    let mut out = Vec::new();
    for name in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
        let Some(doc) = read(name).and_then(|t| t.parse::<toml::Table>().ok()) else { continue };
        if let Some(l) = doc.get("modLoader").and_then(|v| v.as_str()) { out.push(l.to_lowercase()) }
        for deps in doc.get("dependencies").and_then(|d| d.as_table()).into_iter().flat_map(|t| t.values()) {
            for d in deps.as_array().into_iter().flatten() {
                let required = match d.get("type").and_then(|t| t.as_str()) {
                    Some(t) => t.eq_ignore_ascii_case("required"),
                    None => d.get("mandatory").and_then(|m| m.as_bool()).unwrap_or(false),
                };
                if let (true, Some(id)) = (required, d.get("modId").and_then(|m| m.as_str())) { out.push(id.to_lowercase()) }
            }
        }
    }
    if let Some(v) = read("fabric.mod.json").and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
        out.extend(v["depends"].as_object().into_iter().flatten().map(|(k, _)| k.to_lowercase()));
    }
    if nested {
        let inner: Vec<String> = zip.file_names()
            .filter(|n| (n.starts_with("META-INF/jarjar/") || n.starts_with("META-INF/jars/")) && n.ends_with(".jar")).map(str::to_string).collect();
        for n in inner {
            let mut bytes = Vec::new();
            if zip.by_name(&n).ok().and_then(|mut f| f.read_to_end(&mut bytes).ok()).is_some() {
                out.extend(needs_in(std::io::Cursor::new(bytes), false));
            }
        }
    }
    out
}
