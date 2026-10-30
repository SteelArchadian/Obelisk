//! CurseForge API with the user's own key: search, project pages, file installs, fingerprints.
//! Files whose authors disable third-party downloads come back with no URL; those are handed
//! to the user's browser instead (see `wait_for_download`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// CurseForge project id of Fabric API (under Connector, Forgified Fabric API stands in for it).
pub const FABRIC_API: u64 = 306612;

const API: &str = "https://api.curseforge.com/v1";
pub const MINECRAFT: u32 = 432;

pub struct Client {
    agent: ureq::Agent,
    key: String,
}

/// CurseForge classId for a Modrinth-style project type.
pub fn class_id(project_type: &str) -> Option<u32> {
    match project_type {
        "mod" => Some(6),
        "modpack" => Some(4471),
        "resourcepack" => Some(12),
        "shader" => Some(6552),
        "datapack" => Some(6945),
        _ => None,
    }
}

pub fn folder_for_class(class_id: u64) -> Option<&'static str> {
    match class_id {
        6 => Some("mods"),
        12 => Some("resourcepacks"),
        6552 => Some("shaderpacks"),
        // Relative to a world folder, not the profile.
        6945 => Some("datapacks"),
        _ => None,
    }
}

/// CurseForge modLoaderType for a loader name.
pub fn loader_type(loader: &str) -> Option<u32> {
    match loader.to_lowercase().as_str() {
        "forge" => Some(1),
        "fabric" => Some(4),
        "quilt" => Some(5),
        "neoforge" => Some(6),
        _ => None,
    }
}

/// CurseForge's file fingerprint: MurmurHash2 (seed 1) over the file with whitespace bytes removed.
pub fn fingerprint(bytes: &[u8]) -> u32 {
    let data: Vec<u8> = bytes.iter().copied().filter(|b| !matches!(b, 9 | 10 | 13 | 32)).collect();
    const M: u32 = 0x5bd1_e995;
    let len = data.len();
    let mut h: u32 = 1 ^ (len as u32);
    let mut chunks = data.chunks_exact(4);
    for c in &mut chunks {
        let mut k = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        k = k.wrapping_mul(M);
        k ^= k >> 24;
        k = k.wrapping_mul(M);
        h = h.wrapping_mul(M) ^ k;
    }
    let rest = chunks.remainder();
    if rest.len() >= 3 { h ^= (rest[2] as u32) << 16; }
    if rest.len() >= 2 { h ^= (rest[1] as u32) << 8; }
    if !rest.is_empty() {
        h ^= rest[0] as u32;
        h = h.wrapping_mul(M);
    }
    h ^= h >> 13;
    h = h.wrapping_mul(M);
    h ^ (h >> 15)
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub query: String,
    pub project_type: String,
    pub loader: String,
    /// Several loaders at once (any of them); takes over from `loader` when set.
    #[serde(default)]
    pub loaders: Vec<String>,
    pub game_version: String,
    /// relevance, downloads, follows, updated, newest, name
    pub sort: String,
    pub offset: u32,
    pub limit: u32,
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
        _ => format!("%{b:02X}"),
    }).collect()
}

/// Relations: what a project depends on, or what depends on it ("dependents"). The official API has no list of
/// dependents, so this reads the same public, key-free endpoint curseforge.com's Relations tab uses (20 per page).
/// `kind` narrows it: RequiredDependency, OptionalDependency, Include (packs that bundle it), EmbeddedLibrary, Tool,
/// Incompatible; empty for all. `sort`: popular, updated, newest or name (that endpoint's sortField 2, 1, 4, 3).
pub fn relations(mod_id: u64, which: &str, index: u32, kind: &str, sort: &str) -> Result<Value, String> {
    if which != "dependencies" && which != "dependents" { return Err("unknown relation".into()) }
    let tls = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
    let agent = ureq::AgentBuilder::new().user_agent("Obelisk/0.1").tls_connector(Arc::new(tls))
        .timeout(std::time::Duration::from_secs(20)).build();
    let mut url = format!("https://www.curseforge.com/api/v1/mods/{mod_id}/{which}?index={index}&pageSize=20");
    if !kind.is_empty() { url += &format!("&type={}", enc(kind)) }
    url += match sort { "updated" => "&sortField=1&sortOrder=Desc", "newest" => "&sortField=4&sortOrder=Desc", "name" => "&sortField=3&sortOrder=Asc", _ => "&sortField=2&sortOrder=Desc" };
    agent.get(&url).set("Accept", "application/json").call().map_err(|e| format!("CurseForge: {e}"))?
        .into_json().map_err(|e| e.to_string())
}

/// sortField per CurseForge's API (1 Featured, 2 Popularity, 3 LastUpdated, 4 Name, 6 TotalDownloads, 11 ReleasedDate).
fn sort_field(sort: &str) -> u32 {
    match sort {
        "downloads" => 6,
        "follows" => 2,
        "updated" => 3,
        "newest" => 11,
        "name" => 4,
        _ => 1,
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum InstallOutcome {
    /// Installed; display names (root first, then dependencies).
    Done { names: Vec<String> },
    /// The author only allows downloads on curseforge.com: the user downloads it in a browser.
    Blocked { names: Vec<String>, name: String, page: String, file_name: String, sha1: String, size: u64, folder: String, index_toml: String },
}

impl Client {
    pub fn new(key: &str) -> Result<Self, String> {
        let tls = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
        Ok(Client {
            agent: ureq::AgentBuilder::new().user_agent("Obelisk/0.1").tls_connector(Arc::new(tls)).build(),
            key: key.to_string(),
        })
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        self.retrying(|| self.agent.get(&format!("{API}{path}")).set("x-api-key", &self.key).set("Accept", "application/json").call())
    }

    /// Every POST here is a read-only lookup (projects, files, fingerprints), so it's safe to send again.
    fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        self.retrying(|| self.agent.post(&format!("{API}{path}")).set("x-api-key", &self.key).set("Accept", "application/json")
            .send_json(body.clone()))
    }

    /// A dropped connection or a busy server gets two more tries (see modrinth::passing).
    fn retrying(&self, send: impl Fn() -> Result<ureq::Response, ureq::Error>) -> Result<Value, String> {
        let mut wait = 400;
        for attempt in 1.. {
            match send() {
                Ok(r) => return r.into_json().map_err(|e| e.to_string()),
                Err(e) if attempt < 3 && crate::modrinth::passing(&e) => { std::thread::sleep(std::time::Duration::from_millis(wait)); wait *= 3; }
                Err(e) => return Err(format!("CurseForge: {e}")),
            }
        }
        unreachable!()
    }

    pub fn search(&self, q: &SearchQuery) -> Result<Value, String> {
        let class = class_id(&q.project_type).unwrap_or(6);
        let mut path = format!("/mods/search?gameId={MINECRAFT}&classId={class}&searchFilter={}&sortField={}&sortOrder=desc&index={}&pageSize={}",
            enc(&q.query), sort_field(&q.sort), q.offset, q.limit.clamp(1, 50));
        if !q.game_version.is_empty() {
            path += &format!("&gameVersion={}", enc(&q.game_version));
        }
        let types: Vec<u32> = if q.loaders.is_empty() { vec![q.loader.clone()] } else { q.loaders.clone() }
            .iter().filter_map(|l| loader_type(l)).collect();
        if matches!(class, 6 | 4471) {
            match types.as_slice() {
                [] => {}
                [t] => path += &format!("&modLoaderType={t}"),
                many => path += &format!("&modLoaderTypes={}", enc(&format!("[{}]", many.iter().map(u32::to_string).collect::<Vec<_>>().join(",")))),
            }
        }
        self.get(&path)
    }

    /// Everything an author has made for Minecraft (any project type, any game version), most
    /// downloaded first. CurseForge caps one search at 10,000 results; 500 is plenty for one author.
    pub fn author_projects(&self, author_id: u64) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        for index in (0..500).step_by(50) {
            let page = self.get(&format!("/mods/search?gameId={MINECRAFT}&authorId={author_id}&sortField=6&sortOrder=desc&index={index}&pageSize=50"))?;
            let data = page["data"].as_array().cloned().unwrap_or_default();
            let n = data.len();
            out.extend(data);
            if n < 50 {
                break;
            }
        }
        Ok(out)
    }

    pub fn project(&self, id: u64) -> Result<Value, String> {
        Ok(self.get(&format!("/mods/{id}"))?["data"].take())
    }

    /// A Minecraft project's id from its page address slug (curseforge.com/minecraft/<section>/<slug>).
    pub fn id_by_slug(&self, slug: &str) -> Result<Option<u64>, String> {
        let slug: String = slug.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        let v = self.get(&format!("/mods/search?gameId=432&slug={slug}"))?;
        Ok(v["data"].as_array().and_then(|a| a.iter().find(|p| p["slug"] == slug.as_str())).and_then(|p| p["id"].as_u64()))
    }

    pub fn projects(&self, ids: &[u64]) -> Result<Vec<Value>, String> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for chunk in ids.chunks(500) {
            let v = self.post("/mods", serde_json::json!({ "modIds": chunk, "filterPcOnly": true }))?;
            out.extend(v["data"].as_array().cloned().unwrap_or_default());
        }
        Ok(out)
    }

    pub fn description(&self, id: u64) -> Result<String, String> {
        Ok(self.get(&format!("/mods/{id}/description"))?["data"].as_str().unwrap_or_default().to_string())
    }

    /// Files, newest first; optionally only those for a game version and loader.
    pub fn files(&self, id: u64, game_version: &str, loader: &str) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        let mut index = 0;
        loop {
            let mut path = format!("/mods/{id}/files?index={index}&pageSize=50");
            if !game_version.is_empty() {
                path += &format!("&gameVersion={}", enc(game_version));
            }
            if let Some(t) = loader_type(loader) {
                path += &format!("&modLoaderType={t}");
            }
            let v = self.get(&path)?;
            let page = v["data"].as_array().cloned().unwrap_or_default();
            let total = v["pagination"]["totalCount"].as_u64().unwrap_or(0) as usize;
            let n = page.len();
            out.extend(page);
            index += 50;
            if n < 50 || out.len() >= total || out.len() >= 400 {
                break;
            }
        }
        out.sort_by_key(|f| std::cmp::Reverse(f["id"].as_u64().unwrap_or(0)));
        Ok(out)
    }

    pub fn file(&self, mod_id: u64, file_id: u64) -> Result<Value, String> {
        Ok(self.get(&format!("/mods/{mod_id}/files/{file_id}"))?["data"].take())
    }

    pub fn files_batch(&self, ids: &[u64]) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        for chunk in ids.chunks(500) {
            let v = self.post("/mods/files", serde_json::json!({ "fileIds": chunk }))?;
            out.extend(v["data"].as_array().cloned().unwrap_or_default());
        }
        Ok(out)
    }

    /// Featured / popular / recently updated lists (works with keys that can't use search).
    pub fn featured(&self, game_version_type: Option<u64>) -> Result<Value, String> {
        Ok(self.post("/mods/featured", serde_json::json!({ "gameId": MINECRAFT, "excludedModIds": [], "gameVersionTypeId": game_version_type }))?["data"].take())
    }

    /// Replace an installed file with another file of the same project (an update).
    /// Returns Ok(None) when done, Ok(Some(file)) when the new file must be downloaded by hand.
    pub fn replace(&self, game: &Path, kind: &str, old_file: &str, mod_id: u64, file_id: u64) -> Result<Option<Value>, String> {
        let project = self.project(mod_id)?;
        let file = self.file(mod_id, file_id)?;
        if file["downloadUrl"].as_str().is_none_or(|u| u.is_empty()) {
            return Ok(Some(file));
        }
        crate::delete_content(game, kind, old_file, |p| fs::remove_file(p).map_err(|e| e.to_string()))?;
        // An update swaps one file for its own newer one: no duplicate check (an empty index).
        self.place(game, &project, &file, kind, &mut Some(Default::default()), None)?;
        Ok(None)
    }

    pub fn changelog(&self, mod_id: u64, file_id: u64) -> Result<String, String> {
        Ok(self.get(&format!("/mods/{mod_id}/files/{file_id}/changelog"))?["data"].as_str().unwrap_or_default().to_string())
    }

    /// Exact fingerprint matches: fingerprint -> (mod id, file).
    pub fn match_fingerprints(&self, fps: &[u32]) -> Result<Vec<(u32, u64, Value)>, String> {
        if fps.is_empty() {
            return Ok(vec![]);
        }
        let v = self.post(&format!("/fingerprints/{MINECRAFT}"), serde_json::json!({ "fingerprints": fps }))?;
        Ok(v["data"]["exactMatches"].as_array().into_iter().flatten().filter_map(|m| {
            let file = m["file"].clone();
            Some((file["fileFingerprint"].as_u64()? as u32, m["id"].as_u64()?, file))
        }).collect())
    }

    /// Install a CurseForge project's best (or a specific) file plus required dependencies.
    /// `installed` holds CurseForge project ids already present. Dependencies that are
    /// download-blocked are skipped (reported in the names list with a note).
    pub fn install(&self, game: &Path, mod_id: u64, file_id: Option<u64>, loader: &str, mc: &str, installed: &HashSet<u64>) -> Result<InstallOutcome, String> {
        let project = self.project(mod_id)?;
        let candidates = match file_id {
            Some(f) => vec![self.file(mod_id, f)?],
            None => by_preference(self.files(mod_id, mc, if is_mod_class(&project) { loader } else { "" })?),
        };
        if candidates.is_empty() {
            return Err(format!("no file for Minecraft {mc} / {loader}"));
        }
        let folder = folder_for_class(project["classId"].as_u64().unwrap_or(6)).ok_or("this kind of project can't be installed into a profile yet")?;
        // Jars are checked against the pack's loader and Minecraft version before they go in (see fit.rs).
        let pack = (folder == "mods").then(|| crate::fit::pack(&game.join("mods"), loader, &crate::loader_version_of(game), mc));
        let mut names = Vec::new();
        let mut have = None;
        let mut seen = installed.clone();
        seen.insert(mod_id);

        let name = project["name"].as_str().unwrap_or_default().to_string();
        let (placed_root, file) = self.place_first(game, &project, candidates, folder, &mut have, pack.as_ref())?;
        let placed = &placed_root;
        if let Placed::AlreadyThere(other) = placed {
            return Err(format!("{name} is already installed as {other} (probably from the other site); not adding a second copy"));
        }
        if let Placed::Unfit(why) = placed {
            return Err(why.clone());
        }

        // Its required dependencies too (also when the root itself is download-blocked).
        let mut queue = required_deps(&file);
        let mut depth = 0;
        while !queue.is_empty() && depth < 8 {
            let mut next = Vec::new();
            for dep in queue {
                if !seen.insert(dep) {
                    continue;
                }
                let dp = self.project(dep)?;
                let dname = dp["name"].as_str().unwrap_or("dependency").to_string();
                let files = by_preference(self.files(dep, mc, loader)?);
                if files.is_empty() {
                    names.push(format!("{dname} (no {mc} file)"));
                    continue;
                }
                let (placed, df) = self.place_first(game, &dp, files, "mods", &mut have, pack.as_ref())?;
                match placed {
                    Placed::Yes => names.push(dname),
                    Placed::AlreadyThere(_) => {}
                    Placed::Blocked => names.push(format!("{dname} (download it on curseforge.com)")),
                    Placed::Unfit(why) => {
                        // Without it the game would crash, so the mod doesn't stay either.
                        if placed_root == Placed::Yes {
                            let jar = file["fileName"].as_str().unwrap_or_default();
                            crate::delete_content(game, folder, jar, |f| fs::remove_file(f).map_err(|e| e.to_string()))?;
                        }
                        return Err(format!("{name} needs {dname}, but {why}"));
                    }
                }
                next.extend(required_deps(&df));
            }
            queue = next;
            depth += 1;
        }

        if placed_root == Placed::Yes {
            names.insert(0, name);
            Ok(InstallOutcome::Done { names })
        } else {
            let page = format!("{}/files/{}", project["links"]["websiteUrl"].as_str().unwrap_or_default().trim_end_matches('/'), file["id"]);
            Ok(InstallOutcome::Blocked {
                names, name, page,
                file_name: file["fileName"].as_str().unwrap_or_default().to_string(),
                sha1: sha1_of(&file), size: file["fileLength"].as_u64().unwrap_or(0),
                folder: folder.into(), index_toml: index_toml(&project, &file),
            })
        }
    }

    /// Place the first of these files (best first) that loads in the pack; when none does, the newest one's reason.
    fn place_first(&self, game: &Path, project: &Value, files: Vec<Value>, folder: &str,
                   have: &mut Option<std::collections::HashMap<String, Vec<String>>>, pack: Option<&crate::fit::Pack>) -> Result<(Placed, Value), String> {
        let mut first_why = None;
        let tried = files.len();
        for f in files {
            match self.place(game, project, &f, folder, have, pack)? {
                Placed::Unfit(why) => { first_why.get_or_insert(why); }
                placed => return Ok((placed, f)),
            }
        }
        let why = first_why.unwrap_or_default();
        Ok((Placed::Unfit(if tried > 1 { format!("{why} (and the {} files before it don't fit either)", tried - 1) } else { why }), Value::Null))
    }

    /// Download a file into `game/folder` and write its Prism metadata. Blocked when the author
    /// disabled third-party downloads (no URL); AlreadyThere (nothing written) when another jar
    /// already provides the same mod id, e.g. the Modrinth copy. `have` caches the folder's mod ids.
    fn place(&self, game: &Path, project: &Value, file: &Value, folder: &str,
             have: &mut Option<std::collections::HashMap<String, Vec<String>>>, pack: Option<&crate::fit::Pack>) -> Result<Placed, String> {
        let Some(url) = file["downloadUrl"].as_str().filter(|u| !u.is_empty()) else { return Ok(Placed::Blocked) };
        let name = file["fileName"].as_str().unwrap_or_default();
        if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
            return Err("bad file name from CurseForge".into());
        }
        let mut bytes = Vec::new();
        self.agent.get(url).call().map_err(|e| format!("download: {e}"))?
            .into_reader().take(1 << 30).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        let want = sha1_of(file);
        if !want.is_empty() && crate::sha1_hex(&bytes) != want {
            return Err(format!("{name}: hash mismatch"));
        }
        let dir = game.join(folder);
        if let Some(meta) = pack.and_then(|_| crate::fit::read_bundle(&bytes)) {
            let unloadable = crate::fit::check(&meta, pack.unwrap(), &[]).into_iter().find(|p| matches!(p, crate::fit::Problem::Unloadable(_)));
            if let Some(p) = unloadable {
                let label = format!("{} ({name})", project["name"].as_str().unwrap_or_default());
                return Ok(Placed::Unfit(p.describe(&label)));
            }
        }
        if folder == "mods" {
            let ids = crate::dupes::own_ids_of(&bytes);
            let index = have.get_or_insert_with(|| crate::dupes::index(&dir));
            if let Some((_, other)) = crate::dupes::clash(index, &ids, name) {
                return Ok(Placed::AlreadyThere(other));
            }
            for id in ids { index.entry(id).or_default().push(name.to_string()) }
        }
        fs::create_dir_all(dir.join(".index")).map_err(|e| e.to_string())?;
        fs::write(dir.join(name), &bytes).map_err(|e| e.to_string())?;
        write_index(&dir, project, file)?;
        Ok(Placed::Yes)
    }
}

#[derive(Debug, PartialEq)]
enum Placed {
    Yes,
    Blocked,
    AlreadyThere(String),
    /// Won't load in this pack (why).
    Unfit(String),
}

fn is_mod_class(project: &Value) -> bool {
    matches!(project["classId"].as_u64(), Some(6) | Some(4471))
}

fn required_deps(file: &Value) -> Vec<u64> {
    file["dependencies"].as_array().into_iter().flatten()
        .filter(|d| d["relationType"].as_u64() == Some(3))
        .filter_map(|d| d["modId"].as_u64()).collect()
}

/// Newest release file (files come newest first), else newest of any type.
/// Files to try in order: releases newest first, then the rest; at most six.
fn by_preference(mut files: Vec<Value>) -> Vec<Value> {
    files.sort_by_key(|f| f["releaseType"].as_u64() != Some(1)); // stable: keeps newest first within each
    files.truncate(6);
    files
}

pub fn pick_file(files: &[Value]) -> Option<Value> {
    files.iter().find(|f| f["releaseType"].as_u64() == Some(1)).or_else(|| files.first()).cloned()
}

pub fn sha1_of(file: &Value) -> String {
    file["hashes"].as_array().into_iter().flatten()
        .find(|h| h["algo"].as_u64() == Some(1)).and_then(|h| h["value"].as_str())
        .unwrap_or_default().to_lowercase()
}

pub fn index_toml(project: &Value, file: &Value) -> String {
    let q = |s: &str| format!("'{}'", s.replace('\'', ""));
    format!("filename = {}\nname = {}\nside = ''\n\n[download]\nhash = {}\nhash-format = 'sha1'\nmode = 'metadata:curseforge'\nurl = ''\n\n[update.curseforge]\nfile-id = {}\nproject-id = {}\n",
        q(file["fileName"].as_str().unwrap_or_default()), q(project["name"].as_str().unwrap_or_default()),
        q(&sha1_of(file)), file["id"].as_u64().unwrap_or(0), project["id"].as_u64().unwrap_or(0))
}

pub fn write_index(dir: &Path, project: &Value, file: &Value) -> Result<(), String> {
    let slug = project["slug"].as_str().unwrap_or("mod").chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect::<String>();
    fs::create_dir_all(dir.join(".index")).map_err(|e| e.to_string())?;
    fs::write(dir.join(".index").join(format!("{slug}.pw.toml")), index_toml(project, file)).map_err(|e| e.to_string())
}

/// Wait for the user's browser to finish downloading a file (matched by size and SHA-1),
/// then move it into `dest_dir`. Gives up after `timeout`.
pub fn wait_for_download(downloads: &Path, size: u64, sha1: &str, dest: &Path, timeout: Duration) -> Result<PathBuf, String> {
    let start = Instant::now();
    let mut checked: HashSet<(PathBuf, u64)> = HashSet::new();
    while start.elapsed() < timeout {
        if let Ok(entries) = fs::read_dir(downloads) {
            for e in entries.flatten() {
                let p = e.path();
                let name = p.to_string_lossy().to_lowercase();
                if name.ends_with(".crdownload") || name.ends_with(".part") || name.ends_with(".tmp") {
                    continue;
                }
                let Ok(meta) = e.metadata() else { continue };
                let stamp = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0);
                if !meta.is_file() || meta.len() != size || !checked.insert((p.clone(), stamp)) {
                    continue;
                }
                let Ok(bytes) = fs::read(&p) else { continue };
                if sha1.is_empty() || crate::sha1_hex(&bytes) == sha1 {
                    fs::create_dir_all(dest.parent().unwrap_or(dest)).map_err(|e| e.to_string())?;
                    fs::rename(&p, dest).or_else(|_| fs::copy(&p, dest).map(|_| ()).and_then(|_| fs::remove_file(&p)))
                        .map_err(|e| e.to_string())?;
                    return Ok(dest.to_path_buf());
                }
            }
        }
        std::thread::sleep(Duration::from_millis(1500));
    }
    Err("timed out waiting for the download".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_ignores_whitespace() {
        // Same bytes apart from whitespace give the same fingerprint; different content doesn't.
        assert_eq!(fingerprint(b"ab cd\n\tef\r"), fingerprint(b"abcdef"));
        assert_ne!(fingerprint(b"abcdef"), fingerprint(b"abcdeg"));
        assert_eq!(fingerprint(b""), {
            let mut h: u32 = 1;
            h ^= h >> 13; h = h.wrapping_mul(0x5bd1_e995); h ^ (h >> 15)
        });
    }

    #[test]
    fn picks_release_then_any_and_reads_deps() {
        let files = vec![
            serde_json::json!({ "id": 3, "releaseType": 2, "dependencies": [] }),
            serde_json::json!({ "id": 2, "releaseType": 1, "dependencies": [
                { "modId": 7, "relationType": 3 }, { "modId": 8, "relationType": 2 }] }),
        ];
        let f = pick_file(&files).unwrap();
        assert_eq!(f["id"], 2);
        assert_eq!(required_deps(&f), vec![7]);
        assert_eq!(pick_file(&files[..1]).unwrap()["id"], 3);
        assert_eq!(loader_type("NeoForge"), Some(6));
        assert_eq!(class_id("shader"), Some(6552));
        assert_eq!(sort_field("downloads"), 6);
    }

    #[test]
    fn index_toml_reads_back_as_curseforge() {
        let tmp = tempfile::tempdir().unwrap();
        let project = serde_json::json!({ "id": 42, "name": "Jade", "slug": "jade" });
        let file = serde_json::json!({ "id": 9001, "fileName": "jade.jar", "hashes": [{ "algo": 1, "value": "ABC" }] });
        let mods = tmp.path().join("mods");
        fs::create_dir_all(&mods).unwrap();
        fs::write(mods.join("jade.jar"), "x").unwrap();
        write_index(&mods, &project, &file).unwrap();
        let items = crate::list_content(tmp.path(), "mods");
        assert_eq!((items[0].source.as_str(), items[0].project_id.as_str(), items[0].name.as_str()), ("curseforge", "42", "Jade"));
    }

    #[test]
    fn waits_for_matching_download() {
        let tmp = tempfile::tempdir().unwrap();
        let dl = tmp.path().join("dl");
        fs::create_dir_all(&dl).unwrap();
        fs::write(dl.join("wrong.jar"), b"abcd").unwrap(); // same size, wrong hash
        fs::write(dl.join("right.jar"), b"wxyz").unwrap();
        fs::write(dl.join("right.jar.crdownload"), b"wxyz").unwrap();
        let dest = tmp.path().join("mods/x.jar");
        let got = wait_for_download(&dl, 4, &crate::sha1_hex(b"wxyz"), &dest, Duration::from_secs(5)).unwrap();
        assert_eq!(fs::read(got).unwrap(), b"wxyz");
        assert!(dl.join("wrong.jar").exists() && !dl.join("right.jar").exists());
        assert!(wait_for_download(&dl, 99, "", &dest, Duration::from_millis(10)).is_err());
    }
}
