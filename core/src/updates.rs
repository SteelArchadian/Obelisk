//! Where each file in a profile comes from, how popular it is, and whether a newer build exists.
//! Source order: Prism metadata, then exact hash on Modrinth, then exact fingerprint on CurseForge.

use crate::{curseforge, modrinth};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Serialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub kind: String,
    pub file_name: String,
    /// "modrinth", "curseforge" or "" (unknown, e.g. your own mods)
    pub provider: String,
    pub project_id: String,
    /// Modrinth version id or CurseForge file id currently installed.
    pub current: String,
    pub downloads: u64,
    /// "client", "server", "both" or ""
    pub env: String,
    /// The project's icon on its site (used when the file has no icon of its own, e.g. shaders).
    pub icon_url: String,
    /// The site files it as a library or API (Modrinth "library", CurseForge "API and Library").
    pub library: bool,
    pub update: Option<Update>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    /// Modrinth version id or CurseForge file id.
    pub id: String,
    pub name: String,
    pub file_name: String,
    /// CurseForge author disabled third-party downloads for this file.
    pub blocked: bool,
}

fn loaders_for(kind: &str, loader: &str) -> Vec<String> {
    match kind {
        "mods" => vec![loader.to_lowercase()],
        "resourcepacks" => vec!["minecraft".into()],
        _ => vec!["iris".into(), "optifine".into()],
    }
}

/// Prism `.index` entries: base file name -> (provider, project id, current id).
fn index_map(dir: &Path) -> HashMap<String, (String, String, String)> {
    let mut map = HashMap::new();
    let Ok(entries) = fs::read_dir(dir) else { return map };
    for e in entries.flatten() {
        let Ok(doc) = fs::read_to_string(e.path()).unwrap_or_default().parse::<toml::Table>() else { continue };
        let Some(file) = doc.get("filename").and_then(|v| v.as_str()) else { continue };
        let u = doc.get("update");
        if let Some(m) = u.and_then(|u| u.get("modrinth")) {
            map.insert(file.into(), ("modrinth".into(),
                m.get("mod-id").and_then(|v| v.as_str()).unwrap_or_default().into(),
                m.get("version").and_then(|v| v.as_str()).unwrap_or_default().into()));
        } else if let Some(c) = u.and_then(|u| u.get("curseforge")) {
            let num = |k: &str| c.get(k).and_then(|v| v.as_integer()).map(|n| n.to_string()).unwrap_or_default();
            map.insert(file.into(), ("curseforge".into(), num("project-id"), num("file-id")));
        }
    }
    map
}

fn env_from_sides(client: &str, server: &str) -> String {
    match (client != "unsupported", server != "unsupported") {
        (true, true) => "both".into(),
        (true, false) => "client".into(),
        (false, true) => "server".into(),
        _ => String::new(),
    }
}

/// Fingerprints already worked out, by SHA-1 (reading every jar again would take seconds in a big pack).
static FINGERPRINTS: std::sync::Mutex<Option<HashMap<String, u32>>> = std::sync::Mutex::new(None);

/// The profile's CurseForge files, quickly (for "Installed" on project pages): Prism's metadata, then one batched
/// fingerprint match for the files nothing else names. No update lookups.
pub fn cf_installed(game: &Path, hash_cache: &Path, cf: &curseforge::Client) -> Vec<modrinth::Installed> {
    let mut out = Vec::new();
    let mut unknown: Vec<(String, String, String)> = Vec::new(); // kind, file, sha1
    for kind in crate::KINDS {
        let index = index_map(&game.join(kind).join(".index"));
        for (file, sha1) in crate::file_sha1s(game, kind, hash_cache) {
            let base = file.trim_end_matches(".disabled").to_string();
            match index.get(&base) {
                Some((p, id, current)) if p == "curseforge" => out.push(modrinth::Installed {
                    project_id: format!("cf:{id}"), version_id: current.clone(), kind: kind.to_string(), file_name: file }),
                Some(_) => {}
                None => unknown.push((kind.to_string(), file, sha1)),
            }
        }
    }
    let mut fps: HashMap<u32, usize> = HashMap::new();
    {
        let mut known = FINGERPRINTS.lock().unwrap();
        let known = known.get_or_insert_with(HashMap::new);
        for (n, (kind, file, sha1)) in unknown.iter().enumerate() {
            let fp = match known.get(sha1) {
                Some(&fp) => fp,
                None => {
                    let Ok(bytes) = fs::read(game.join(kind).join(file)) else { continue };
                    let fp = curseforge::fingerprint(&bytes);
                    known.insert(sha1.clone(), fp);
                    fp
                }
            };
            fps.insert(fp, n);
        }
    }
    if fps.is_empty() {
        return out;
    }
    for (fp, mod_id, file) in cf.match_fingerprints(&fps.keys().copied().collect::<Vec<_>>()).unwrap_or_default() {
        if let Some(&n) = fps.get(&fp) {
            let (kind, name, _) = &unknown[n];
            out.push(modrinth::Installed { project_id: format!("cf:{mod_id}"), version_id: file["id"].as_u64().unwrap_or(0).to_string(),
                kind: kind.clone(), file_name: name.clone() });
        }
    }
    out
}

/// Look at every file in a profile. Network: a handful of batched calls, not one per mod.
pub fn check(game: &Path, hash_cache: &Path, mr: &modrinth::Client, cf: Option<&curseforge::Client>, loader: &str, mc: &str) -> Result<Vec<FileInfo>, String> {
    let mut infos: Vec<FileInfo> = Vec::new();
    let mut sha_of: HashMap<(String, String), String> = HashMap::new();

    for kind in crate::KINDS {
        let index = index_map(&game.join(kind).join(".index"));
        for (file, sha1) in crate::file_sha1s(game, kind, hash_cache) {
            let base = file.trim_end_matches(".disabled").to_string();
            let (provider, project_id, current) = index.get(&base).cloned().unwrap_or_default();
            sha_of.insert((kind.into(), file.clone()), sha1);
            infos.push(FileInfo { kind: kind.into(), file_name: file, provider, project_id, current, ..Default::default() });
        }
    }

    // Unknown files: exact hash on Modrinth, then fingerprint on CurseForge.
    let unknown_hashes: Vec<String> = infos.iter().filter(|i| i.provider.is_empty())
        .map(|i| sha_of[&(i.kind.clone(), i.file_name.clone())].clone()).collect();
    let by_hash = mr.lookup_hashes(&unknown_hashes).unwrap_or_default();
    for i in infos.iter_mut().filter(|i| i.provider.is_empty()) {
        if let Some(v) = by_hash.get(&sha_of[&(i.kind.clone(), i.file_name.clone())]) {
            i.provider = "modrinth".into();
            i.project_id = v.project_id.clone();
            i.current = v.id.clone();
        }
    }
    if let Some(cf) = cf {
        let mut fps: HashMap<u32, usize> = HashMap::new();
        for (n, i) in infos.iter().enumerate().filter(|(_, i)| i.provider.is_empty()) {
            if let Ok(bytes) = fs::read(game.join(&i.kind).join(&i.file_name)) {
                fps.insert(curseforge::fingerprint(&bytes), n);
            }
        }
        for (fp, mod_id, file) in cf.match_fingerprints(&fps.keys().copied().collect::<Vec<_>>()).unwrap_or_default() {
            if let Some(&n) = fps.get(&fp) {
                infos[n].provider = "curseforge".into();
                infos[n].project_id = mod_id.to_string();
                infos[n].current = file["id"].as_u64().unwrap_or(0).to_string();
            }
        }
    }

    // Modrinth: newest compatible version per file, plus downloads and client/server sides.
    for kind in crate::KINDS {
        let hashes: Vec<String> = infos.iter().filter(|i| i.kind == kind && i.provider == "modrinth")
            .map(|i| sha_of[&(i.kind.clone(), i.file_name.clone())].clone()).collect();
        let latest = mr.latest_for_hashes(&hashes, &loaders_for(kind, loader), &[mc.to_string()]).unwrap_or_default();
        for i in infos.iter_mut().filter(|i| i.kind == kind && i.provider == "modrinth") {
            let Some(v) = latest.get(&sha_of[&(i.kind.clone(), i.file_name.clone())]) else { continue };
            if v.id != i.current && v.project_id == i.project_id {
                let f = modrinth::primary_file(v);
                i.update = Some(Update { id: v.id.clone(), name: v.version_number.clone(),
                    file_name: f.map(|f| f.filename.clone()).unwrap_or_default(), blocked: false });
            }
        }
    }
    let mr_ids: Vec<String> = unique(infos.iter().filter(|i| i.provider == "modrinth").map(|i| i.project_id.clone()));
    for p in mr.projects(&mr_ids).unwrap_or_default() {
        let id = p["id"].as_str().unwrap_or_default();
        for i in infos.iter_mut().filter(|i| i.provider == "modrinth" && i.project_id == id) {
            i.downloads = p["downloads"].as_u64().unwrap_or(0);
            i.icon_url = p["icon_url"].as_str().unwrap_or_default().to_string();
            i.env = env_from_sides(p["client_side"].as_str().unwrap_or_default(), p["server_side"].as_str().unwrap_or_default());
            i.library = p["categories"].as_array().is_some_and(|c| c.iter().any(|c| c == "library"));
        }
    }

    // CurseForge: newest compatible file from each project's latest-files index.
    if let Some(cf) = cf {
        let ids: Vec<u64> = unique(infos.iter().filter(|i| i.provider == "curseforge").filter_map(|i| i.project_id.parse().ok()));
        let projects = cf.projects(&ids).unwrap_or_default();
        let lt = curseforge::loader_type(loader);
        let mut wanted: Vec<u64> = Vec::new();
        for p in &projects {
            let id = p["id"].as_u64().unwrap_or(0).to_string();
            let is_mod = p["classId"].as_u64() == Some(6);
            let best = best_latest(p, mc, if is_mod { lt } else { None });
            for i in infos.iter_mut().filter(|i| i.provider == "curseforge" && i.project_id == id) {
                i.downloads = p["downloadCount"].as_f64().unwrap_or(0.0) as u64;
                // Some logos have an empty thumbnail: the full logo then.
                i.icon_url = [&p["logo"]["thumbnailUrl"], &p["logo"]["url"]].into_iter().filter_map(|v| v.as_str()).find(|u| !u.is_empty()).unwrap_or_default().to_string();
                i.library = p["categories"].as_array().is_some_and(|c| c.iter().any(|c| c["slug"] == "library-api" || c["id"] == 421));
                let current: u64 = i.current.parse().unwrap_or(0);
                if let Some(best) = best.filter(|b| *b > current) {
                    i.update = Some(Update { id: best.to_string(), ..Default::default() });
                    wanted.push(best);
                }
            }
        }
        // Client/server tags come from the installed file itself.
        let current: Vec<u64> = infos.iter().filter(|i| i.provider == "curseforge").filter_map(|i| i.current.parse().ok()).collect();
        wanted.extend(current);
        let files: HashMap<String, Value> = cf.files_batch(&wanted).unwrap_or_default().into_iter()
            .map(|f| (f["id"].as_u64().unwrap_or(0).to_string(), f)).collect();
        let env_of = |f: &Value| {
            let gv: Vec<&str> = f["gameVersions"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).collect();
            match (gv.contains(&"Client"), gv.contains(&"Server")) {
                (true, true) => "both", (true, false) => "client", (false, true) => "server", _ => "",
            }.to_string()
        };
        for i in infos.iter_mut().filter(|i| i.provider == "curseforge") {
            if let Some(f) = files.get(&i.current) {
                i.env = env_of(f);
            }
            if let Some(u) = i.update.as_mut() {
                if let Some(f) = files.get(&u.id) {
                    u.name = f["displayName"].as_str().unwrap_or_default().to_string();
                    u.file_name = f["fileName"].as_str().unwrap_or_default().to_string();
                    u.blocked = f["downloadUrl"].as_str().is_none_or(|s| s.is_empty());
                }
            }
        }
    }
    Ok(infos)
}

/// Highest file id in a CurseForge project's latest-files index for this game version (and loader),
/// preferring releases.
pub fn best_latest(project: &Value, mc: &str, loader_type: Option<u32>) -> Option<u64> {
    let entries: Vec<&Value> = project["latestFilesIndexes"].as_array().into_iter().flatten()
        .filter(|e| e["gameVersion"].as_str() == Some(mc))
        .filter(|e| loader_type.is_none() || e["modLoader"].as_u64() == loader_type.map(u64::from))
        .collect();
    let pick = |release_only: bool| entries.iter()
        .filter(|e| !release_only || e["releaseType"].as_u64() == Some(1))
        .filter_map(|e| e["fileId"].as_u64()).max();
    pick(true).or_else(|| pick(false))
}

fn unique<T: Eq + std::hash::Hash + Clone>(it: impl Iterator<Item = T>) -> Vec<T> {
    let mut seen = std::collections::HashSet::new();
    it.filter(|x| seen.insert(x.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_latest_prefers_release_for_version_and_loader() {
        let p = serde_json::json!({ "latestFilesIndexes": [
            { "gameVersion": "1.21.1", "fileId": 30, "modLoader": 6, "releaseType": 2 },
            { "gameVersion": "1.21.1", "fileId": 20, "modLoader": 6, "releaseType": 1 },
            { "gameVersion": "1.21.1", "fileId": 40, "modLoader": 4, "releaseType": 1 },
            { "gameVersion": "1.20.1", "fileId": 50, "modLoader": 6, "releaseType": 1 }
        ]});
        assert_eq!(best_latest(&p, "1.21.1", Some(6)), Some(20));
        assert_eq!(best_latest(&p, "1.21.1", None), Some(40));
        assert_eq!(best_latest(&p, "1.19.2", Some(6)), None);
        let betas = serde_json::json!({ "latestFilesIndexes": [{ "gameVersion": "1.21.1", "fileId": 9, "modLoader": 6, "releaseType": 3 }] });
        assert_eq!(best_latest(&betas, "1.21.1", Some(6)), Some(9));
    }

    #[test]
    fn index_map_reads_both_providers() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.pw.toml"), "filename = 'a.jar'\n[update.modrinth]\nmod-id = 'P'\nversion = 'V'\n").unwrap();
        fs::write(tmp.path().join("b.pw.toml"), "filename = 'b.jar'\n[update.curseforge]\nproject-id = 42\nfile-id = 7\n").unwrap();
        let m = index_map(tmp.path());
        assert_eq!(m["a.jar"], ("modrinth".into(), "P".into(), "V".into()));
        assert_eq!(m["b.jar"], ("curseforge".into(), "42".into(), "7".into()));
        assert_eq!(env_from_sides("required", "unsupported"), "client");
    }
}
