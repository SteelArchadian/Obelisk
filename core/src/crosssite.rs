//! The same project on the other site: a CurseForge project's Modrinth twin and the other way round, so a project
//! page shows both sites (and CurseForge-only data such as dependents) however it was opened. The proof is a file
//! both sites host byte for byte (same SHA-1); a matching slug or title is only a fallback.

use crate::{curseforge, modrinth};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

/// Title reduced to letters and digits, without bracketed notes ("Farmer's Delight [Forge]" → "farmersdelight").
pub fn same_key(title: &str) -> String {
    let mut out = String::new();
    let mut depth = 0i32;
    for c in title.to_lowercase().chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = (depth - 1).max(0),
            c if depth == 0 && c.is_ascii_alphanumeric() => out.push(c),
            _ => {}
        }
    }
    out
}

/// SHA-1s of a CurseForge project's files (its latest files, which the project record carries).
fn cf_sha1s(files: &[Value]) -> Vec<String> {
    files.iter().flat_map(|f| f["hashes"].as_array().cloned().unwrap_or_default())
        .filter(|h| h["algo"] == 1).filter_map(|h| h["value"].as_str().map(str::to_lowercase)).collect()
}

/// The Modrinth project hosting any of these files, if they all agree on one.
fn mr_project_for(mr: &modrinth::Client, sha1s: &[String]) -> Result<Option<String>, String> {
    let found = mr.lookup_hashes(sha1s)?;
    let mut count: HashMap<String, usize> = HashMap::new();
    for v in found.values() { *count.entry(v.project_id.clone()).or_default() += 1; }
    Ok(count.into_iter().max_by_key(|(_, n)| *n).map(|(id, _)| id))
}

static CACHE: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

fn cached(key: &str, f: impl FnOnce() -> Result<Option<String>, String>) -> Result<Option<String>, String> {
    if let Some(v) = CACHE.lock().unwrap().get_or_insert_with(HashMap::new).get(key) { return Ok(v.clone()) }
    let v = f()?;
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(key.to_string(), v.clone());
    Ok(v)
}

/// A CurseForge project's Modrinth id.
pub fn cf_to_mr(cf: &curseforge::Client, mr: &modrinth::Client, cf_id: u64) -> Result<Option<String>, String> {
    cached(&format!("cf:{cf_id}"), || {
        let p = cf.project(cf_id)?;
        let mut sha1s = cf_sha1s(p["latestFiles"].as_array().map(Vec::as_slice).unwrap_or_default());
        sha1s.truncate(20);
        if let Some(id) = mr_project_for(mr, &sha1s)? { return Ok(Some(id)) }
        // No shared file: the same slug and title on Modrinth.
        let (slug, key) = (p["slug"].as_str().unwrap_or_default(), same_key(p["name"].as_str().unwrap_or_default()));
        Ok(mr.project(slug).ok().filter(|m| same_key(m["title"].as_str().unwrap_or_default()) == key)
            .and_then(|m| m["id"].as_str().map(String::from)))
    })
}

/// A Modrinth project's CurseForge id.
pub fn mr_to_cf(cf: &curseforge::Client, mr: &modrinth::Client, mr_id: &str) -> Result<Option<u64>, String> {
    let r = cached(&format!("mr:{mr_id}"), || {
        let p = mr.project(mr_id)?;
        let id = p["id"].as_str().unwrap_or(mr_id).to_string();
        let (slug, title) = (p["slug"].as_str().unwrap_or_default(), p["title"].as_str().unwrap_or_default());
        let key = same_key(title);
        let class = match p["project_type"].as_str().unwrap_or("mod") { "resourcepack" => "resourcepack", "shader" => "shader", "modpack" => "modpack", _ => "mod" };
        // Candidates: the same slug, then a title search.
        let mut cands: Vec<Value> = vec![];
        if let Some(c) = cf.id_by_slug(slug)? { cands.push(cf.project(c)?) }
        let q = curseforge::SearchQuery { query: title.to_string(), project_type: class.into(), sort: "relevance".into(), limit: 10, ..Default::default() };
        cands.extend(cf.search(&q)?["data"].as_array().cloned().unwrap_or_default());
        // A candidate sharing a file with this Modrinth project is the one.
        for c in &cands {
            let mut sha1s = cf_sha1s(c["latestFiles"].as_array().map(Vec::as_slice).unwrap_or_default());
            sha1s.truncate(20);
            if mr_project_for(mr, &sha1s)?.as_deref() == Some(id.as_str()) { return Ok(c["id"].as_u64().map(|i| i.to_string())) }
        }
        // Otherwise the same slug with the same title, or the only search hit with the same title.
        let same: Vec<&Value> = cands.iter().filter(|c| same_key(c["name"].as_str().unwrap_or_default()) == key).collect();
        if let Some(c) = same.iter().find(|c| c["slug"] == slug) { return Ok(c["id"].as_u64().map(|i| i.to_string())) }
        let mut ids: Vec<u64> = same.iter().filter_map(|c| c["id"].as_u64()).collect();
        ids.dedup();
        Ok(if ids.len() == 1 { Some(ids[0].to_string()) } else { None })
    })?;
    Ok(r.and_then(|s| s.parse().ok()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn titles_match_across_sites() {
        assert_eq!(super::same_key("Farmer's Delight [Forge]"), "farmersdelight");
        assert_eq!(super::same_key("Farmer’s Delight (NeoForge)"), "farmersdelight");
    }

    /// Live: cargo test --lib crosssite -- --ignored (uses the CurseForge key saved in Obelisk).
    #[test]
    #[ignore]
    fn farmers_delight_both_ways() {
        let key = keyring::Entry::new("Obelisk", "curseforge-api-key").unwrap().get_password().unwrap();
        let (cf, mr) = (crate::curseforge::Client::new(&key).unwrap(), crate::modrinth::Client::new().unwrap());
        let mr_id = super::cf_to_mr(&cf, &mr, 398521).unwrap();
        let cf_id = super::mr_to_cf(&cf, &mr, "farmers-delight").unwrap();
        let aqua = super::mr_to_cf(&cf, &mr, "aquaculture-delight").unwrap();
        println!("cf 398521 -> mr {mr_id:?}; mr farmers-delight -> cf {cf_id:?}; aquaculture-delight -> cf {aqua:?}");
        assert!(mr_id.is_some() && cf_id == Some(398521));
    }
}

#[cfg(test)]
mod connector_tests {
    /// Live: cargo test --lib connector -- --ignored. A Fabric-only CurseForge mod onto NeoForge through Connector.
    #[test]
    #[ignore]
    fn curseforge_fabric_mod_via_connector() {
        let key = keyring::Entry::new("Obelisk", "curseforge-api-key").unwrap().get_password().unwrap();
        let cf = crate::curseforge::Client::new(&key).unwrap();
        let q = crate::curseforge::SearchQuery { query: "Camera Unbound".into(), project_type: "mod".into(), sort: "relevance".into(), limit: 5, ..Default::default() };
        let hits = cf.search(&q).unwrap();
        let id = hits["data"].as_array().unwrap().iter().find(|h| h["name"] == "Camera Unbound").unwrap()["id"].as_u64().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let skip = std::collections::HashSet::from([crate::curseforge::FABRIC_API]);
        let out = cf.install(tmp.path(), id, None, "fabric", "1.21.1", &skip).unwrap();
        let files: Vec<_> = std::fs::read_dir(tmp.path().join("mods")).unwrap().map(|e| e.unwrap().file_name()).collect();
        println!("{out:?} {files:?}");
        assert!(!files.is_empty());
    }
}
