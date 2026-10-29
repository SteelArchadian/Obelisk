//! Modrinth: search, versions and installs that Prism can track (via `.index/<slug>.pw.toml`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

const API: &str = "https://api.modrinth.com/v2";
const USER_AGENT: &str = "Obelisk/0.1 (Prism Launcher front end)";

pub struct Client {
    agent: ureq::Agent,
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub query: String,
    /// mod, modpack, resourcepack, shader, datapack
    pub project_type: String,
    pub loaders: Vec<String>,
    pub game_versions: Vec<String>,
    pub categories: Vec<String>,
    /// relevance, downloads, follows, newest, updated
    pub sort: String,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct VersionFile {
    pub url: String,
    pub filename: String,
    pub hashes: HashMap<String, String>,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub size: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Dependency {
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub dependency_type: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    pub version_type: String,
    #[serde(default)]
    pub loaders: Vec<String>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    pub files: Vec<VersionFile>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub changelog: Option<String>,
    #[serde(default)]
    pub date_published: String,
    #[serde(default)]
    pub downloads: u64,
}

/// Modrinth `facets` parameter: AND of OR-groups.
pub fn facets(q: &SearchQuery) -> String {
    let mut groups: Vec<Vec<String>> = Vec::new();
    if !q.project_type.is_empty() {
        groups.push(vec![format!("project_type:{}", q.project_type)]);
    }
    if !q.loaders.is_empty() {
        groups.push(q.loaders.iter().map(|l| format!("categories:{l}")).collect());
    }
    if !q.game_versions.is_empty() {
        groups.push(q.game_versions.iter().map(|v| format!("versions:{v}")).collect());
    }
    for c in &q.categories {
        groups.push(vec![format!("categories:{c}")]);
    }
    serde_json::to_string(&groups).unwrap_or_else(|_| "[]".into())
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Content folder for a Modrinth project type.
pub fn folder_for(project_type: &str) -> Option<&'static str> {
    match project_type {
        "mod" => Some("mods"),
        "resourcepack" => Some("resourcepacks"),
        "shader" => Some("shaderpacks"),
        // Relative to a world folder, not the profile.
        "datapack" => Some("datapacks"),
        _ => None,
    }
}

/// Which Modrinth loaders match an instance for a given project type.
pub fn loaders_for(project_type: &str, instance_loader: &str) -> Vec<String> {
    match project_type {
        "mod" | "modpack" => vec![instance_loader.to_lowercase()],
        "resourcepack" => vec!["minecraft".into()],
        "shader" => vec!["iris".into(), "optifine".into()],
        "datapack" => vec!["datapack".into()],
        _ => vec![],
    }
}

/// Prefer the newest release; fall back to the newest beta/alpha. Modrinth lists newest first.
pub fn pick_version(versions: &[Version]) -> Option<&Version> {
    versions.iter().find(|v| v.version_type == "release").or_else(|| versions.first())
}

pub fn primary_file(v: &Version) -> Option<&VersionFile> {
    v.files.iter().find(|f| f.primary).or_else(|| v.files.first())
}

fn toml_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', ""))
}

/// Prism/packwiz metadata so Prism shows the mod as Modrinth-managed and can update it.
pub fn index_toml(name: &str, v: &Version, f: &VersionFile) -> String {
    let sha512 = f.hashes.get("sha512").cloned().unwrap_or_default();
    format!(
        "filename = {}\nname = {}\nside = 'both'\nx-prismlauncher-loaders = [{}]\nx-prismlauncher-mc-versions = [{}]\n\
         x-prismlauncher-release-type = {}\nx-prismlauncher-version-number = {}\n\n\
         [download]\nhash = {}\nhash-format = 'sha512'\nmode = 'url'\nurl = {}\n\n\
         [update.modrinth]\nmod-id = {}\nversion = {}\n",
        toml_str(&f.filename), toml_str(name),
        v.loaders.iter().map(|l| toml_str(l)).collect::<Vec<_>>().join(", "),
        v.game_versions.iter().map(|g| toml_str(g)).collect::<Vec<_>>().join(", "),
        toml_str(&v.version_type), toml_str(&v.version_number),
        toml_str(&sha512), toml_str(&f.url), toml_str(&v.project_id), toml_str(&v.id),
    )
}

/// Modrinth project ids already tracked in an instance folder's `.index`.
pub fn installed_ids(game: &Path) -> HashSet<String> {
    let mut ids = HashSet::new();
    for kind in crate::KINDS {
        let Ok(entries) = fs::read_dir(game.join(kind).join(".index")) else { continue };
        for e in entries.flatten() {
            let Ok(doc) = fs::read_to_string(e.path()).unwrap_or_default().parse::<toml::Table>() else { continue };
            if let Some(id) = doc.get("update").and_then(|u| u.get("modrinth")).and_then(|m| m.get("mod-id")).and_then(|v| v.as_str()) {
                ids.insert(id.to_string());
            }
        }
    }
    ids
}

fn safe_slug(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect()
}

impl Client {
    pub fn new() -> Result<Self, String> {
        let tls = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
        Ok(Client {
            agent: ureq::AgentBuilder::new().user_agent(USER_AGENT).tls_connector(Arc::new(tls)).build(),
        })
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        self.agent.get(&format!("{API}{path}")).call()
            .map_err(|e| format!("Modrinth: {e}"))?
            .into_json().map_err(|e| e.to_string())
    }

    pub fn search(&self, q: &SearchQuery) -> Result<Value, String> {
        let sort = if q.sort.is_empty() { "relevance" } else { &q.sort };
        let limit = if q.limit == 0 { 20 } else { q.limit.min(100) };
        self.get(&format!("/search?query={}&facets={}&index={}&offset={}&limit={limit}",
                          enc(&q.query), enc(&facets(q)), enc(sort), q.offset))
    }

    pub fn project(&self, id: &str) -> Result<Value, String> {
        self.get(&format!("/project/{}", enc(id)))
    }

    /// Categories, loaders and release game versions in one call each.
    pub fn tags(&self) -> Result<Value, String> {
        let versions: Vec<Value> = self.get("/tag/game_version")?.as_array().cloned().unwrap_or_default()
            .into_iter().filter(|v| v["version_type"] == "release").collect();
        Ok(serde_json::json!({
            "categories": self.get("/tag/category")?,
            "loaders": self.get("/tag/loader")?,
            "gameVersions": versions.iter().filter_map(|v| v["version"].as_str()).collect::<Vec<_>>(),
        }))
    }

    pub fn versions(&self, project: &str, loaders: &[String], game_versions: &[String], changelog: bool) -> Result<Vec<Version>, String> {
        let mut path = format!("/project/{}/version?include_changelog={changelog}", enc(project));
        if !loaders.is_empty() {
            path += &format!("&loaders={}", enc(&serde_json::to_string(loaders).unwrap()));
        }
        if !game_versions.is_empty() {
            path += &format!("&game_versions={}", enc(&serde_json::to_string(game_versions).unwrap()));
        }
        serde_json::from_value(self.get(&path)?).map_err(|e| e.to_string())
    }

    /// Plain GET of any URL (loader installers and the like), capped at 256 MB.
    pub fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        self.agent.get(url).call().map_err(|e| format!("{url}: {e}"))?
            .into_reader().take(1 << 28).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        Ok(bytes)
    }

    pub fn version(&self, id: &str) -> Result<Version, String> {
        serde_json::from_value(self.get(&format!("/version/{}", enc(id)))?).map_err(|e| e.to_string())
    }

    /// sha1 -> the Modrinth version that contains exactly that file.
    pub fn lookup_hashes(&self, sha1s: &[String]) -> Result<HashMap<String, Version>, String> {
        if sha1s.is_empty() {
            return Ok(HashMap::new());
        }
        let body = serde_json::json!({ "hashes": sha1s, "algorithm": "sha1" });
        let v: Value = self.agent.post(&format!("{API}/version_files")).send_json(body)
            .map_err(|e| format!("Modrinth: {e}"))?.into_json().map_err(|e| e.to_string())?;
        serde_json::from_value(v).map_err(|e| e.to_string())
    }

    /// For each sha1: the newest version for these loaders and game versions.
    pub fn latest_for_hashes(&self, sha1s: &[String], loaders: &[String], game_versions: &[String]) -> Result<HashMap<String, Version>, String> {
        if sha1s.is_empty() {
            return Ok(HashMap::new());
        }
        let body = serde_json::json!({ "hashes": sha1s, "algorithm": "sha1", "loaders": loaders, "game_versions": game_versions });
        let v: Value = self.agent.post(&format!("{API}/version_files/update")).send_json(body)
            .map_err(|e| format!("Modrinth: {e}"))?.into_json().map_err(|e| e.to_string())?;
        serde_json::from_value(v).map_err(|e| e.to_string())
    }

    /// Several projects at once (downloads, sides, icons).
    pub fn projects(&self, ids: &[String]) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        for chunk in ids.chunks(200) {
            let v = self.get(&format!("/projects?ids={}", enc(&serde_json::to_string(chunk).unwrap())))?;
            out.extend(v.as_array().cloned().unwrap_or_default());
        }
        Ok(out)
    }

    pub fn user(&self, name: &str) -> Result<Value, String> {
        self.get(&format!("/user/{}", enc(name)))
    }

    pub fn user_projects(&self, name: &str) -> Result<Value, String> {
        self.get(&format!("/user/{}/projects", enc(name)))
    }

    pub fn members(&self, project: &str) -> Result<Value, String> {
        self.get(&format!("/project/{}/members", enc(project)))
    }

    /// Every project any of this project's versions depends on (Modrinth has no list of dependents).
    pub fn dependencies(&self, project: &str) -> Result<Value, String> {
        self.get(&format!("/project/{}/dependencies", enc(project)))
    }

    // ---- Signed in: a personal access token the user made on modrinth.com (Settings → PATs) ----

    fn authed(&self, method: &str, path: &str, token: &str) -> Result<Option<Value>, String> {
        let r = self.agent.request(method, &format!("{API}{path}")).set("Authorization", token).call()
            .map_err(|e| match e {
                ureq::Error::Status(401, _) => "Modrinth didn't accept the token. Make a new one in Modrinth → Settings → PATs.".to_string(),
                e => format!("Modrinth: {e}"),
            })?;
        if r.status() == 204 { return Ok(None) }
        r.into_json().map(Some).map_err(|e| e.to_string())
    }

    /// The token's account: id, username, avatar.
    pub fn me(&self, token: &str) -> Result<Value, String> {
        self.authed("GET", "/user", token)?.ok_or_else(|| "Modrinth sent no account".into())
    }

    /// Projects the account follows.
    pub fn follows(&self, token: &str, user_id: &str) -> Result<Value, String> {
        Ok(self.authed("GET", &format!("/user/{}/follows", enc(user_id)), token)?.unwrap_or(Value::Array(vec![])))
    }

    pub fn follow(&self, token: &str, project: &str, on: bool) -> Result<(), String> {
        self.authed(if on { "POST" } else { "DELETE" }, &format!("/project/{}/follow", enc(project)), token).map(|_| ())
    }

    pub fn notifications(&self, token: &str, user_id: &str) -> Result<Value, String> {
        Ok(self.authed("GET", &format!("/user/{}/notifications", enc(user_id)), token)?.unwrap_or(Value::Array(vec![])))
    }

    pub fn mark_read(&self, token: &str, ids: &[String]) -> Result<(), String> {
        if ids.is_empty() { return Ok(()) }
        self.authed("PATCH", &format!("/notifications?ids={}", enc(&serde_json::to_string(ids).unwrap())), token).map(|_| ())
    }

    /// Everything in an instance that maps to a Modrinth project: Prism's `.index` entries plus
    /// any other file whose exact hash Modrinth knows (e.g. mods that came from CurseForge).
    pub fn installed(&self, game: &Path, hash_cache: &Path) -> Result<Vec<Installed>, String> {
        let mut out = Vec::new();
        let mut covered = HashSet::new();
        for kind in crate::KINDS {
            let folder = game.join(kind);
            for (base, (project_id, version_id)) in index_entries(&folder.join(".index")) {
                let file_name = [base.clone(), format!("{base}.disabled")].into_iter().find(|f| folder.join(f).is_file());
                if let Some(file_name) = file_name {
                    covered.insert((kind, file_name.clone()));
                    out.push(Installed { project_id, version_id, kind: kind.into(), file_name });
                }
            }
        }
        let mut pending: Vec<(&str, String, String)> = Vec::new();
        for kind in crate::KINDS {
            for (file, sha1) in crate::file_sha1s(game, kind, hash_cache) {
                if !covered.contains(&(kind, file.clone())) {
                    pending.push((kind, file, sha1));
                }
            }
        }
        let found = self.lookup_hashes(&pending.iter().map(|p| p.2.clone()).collect::<Vec<_>>())?;
        for (kind, file_name, sha1) in pending {
            if let Some(v) = found.get(&sha1) {
                out.push(Installed { project_id: v.project_id.clone(), version_id: v.id.clone(), kind: kind.into(), file_name });
            }
        }
        Ok(out)
    }

    /// Install a project (and its required dependencies) into an instance. `existing` is what
    /// `installed()` returned; an installed project gets its old file replaced. Returns display names.
    pub fn install(&self, game: &Path, req: &InstallRequest, loader: &str, mc: &str, existing: &[Installed]) -> Result<Vec<String>, String> {
        use crate::fit::Problem;
        let use_loader = req.loader_override.clone().unwrap_or_else(|| loader.to_lowercase());
        let cross = use_loader != loader.to_lowercase();
        let folder = folder_for(&req.project_type).ok_or_else(|| format!("can't install {} into an instance yet", req.project_type))?;
        // Only jars in mods/ answer to the loader; resource packs, shaders and data packs aren't checked.
        let checking = folder == "mods";
        let pack = if checking { crate::fit::pack(&game.join(folder), loader, &crate::loader_version_of(game), mc) } else { Default::default() };
        let by_project: HashMap<&str, &Installed> = existing.iter().map(|i| (i.project_id.as_str(), i)).collect();
        let ids_of = |i: Option<&Installed>| i.map(|i| crate::dupes::own_ids(&game.join(&i.kind).join(&i.file_name))).unwrap_or_default();
        let ctx = Ctx { project_type: &req.project_type, loader: &use_loader, mc, pack: &pack, checking };

        let old_root = by_project.get(req.project_id.as_str()).copied();
        let mut root = self.choose(&ctx, &req.project_id, req.version_id.as_deref(), None, &ids_of(old_root))?
            .ok_or("nothing to install")?;
        root.replaces = old_root.cloned();
        let mut plan = vec![root];
        let mut planned: HashSet<String> = HashSet::from([req.project_id.clone()]);
        let mut soft = Vec::new();

        // Breadth-first over what each new jar needs: Modrinth's required dependencies, then whatever the jar itself
        // declares (a library Modrinth doesn't list, or a newer version of one the pack has).
        let mut level = vec![0];
        let mut depth = 0;
        while !level.is_empty() && depth < 8 {
            let mut next = Vec::new();
            for at in level {
                for dep in deps_of(&plan[at].v) {
                    if planned.contains(&dep) || (cross && dep == FABRIC_API) {
                        continue; // under Connector, Forgified Fabric API stands in for Fabric API
                    }
                    planned.insert(dep.clone());
                    let have = by_project.get(dep.as_str()).copied();
                    // Already in the pack: brought up to the newest version for this game version and loader,
                    // since a new mod is usually built against the newest of its libraries.
                    let pick = self.choose(&ctx, &dep, None, have.map(|i| i.version_id.as_str()), &ids_of(have))
                        .map_err(|e| format!("{} needs another mod: {e}", plan[at].name))?;
                    if let Some(mut p) = pick {
                        p.replaces = have.cloned();
                        p.dep = true;
                        plan.push(p);
                        next.push(plan.len() - 1);
                    }
                }
                let Some(meta) = plan[at].meta.clone() else { continue };
                let fabric = meta.kind == "fabric";
                let name = plan[at].name.clone();
                for problem in crate::fit::check(&meta, &pack, &ids_of(plan[at].replaces.as_ref())) {
                    match &problem {
                        Problem::Unloadable(_) => {} // choose() never returns one of these
                        Problem::Breaks { .. } => soft.push(problem.describe(&name)),
                        Problem::Missing { id, range } | Problem::Outdated { id, range, .. } => {
                            if provided(&plan, id, range, &meta.kind) { continue }
                            if fabric {
                                soft.push(problem.describe(&name)); // Connector's world: ids and versions don't map one to one
                                continue;
                            }
                            let found = match &problem {
                                Problem::Missing { .. } => self.find_mod(&ctx, id)?,
                                _ => tracked_owner(game, folder, existing, id),
                            };
                            let pick = match found {
                                Some(project) if !planned.contains(&project) => {
                                    let have = by_project.get(project.as_str()).copied();
                                    let p = self.choose(&ctx, &project, None, have.map(|i| i.version_id.as_str()), &ids_of(have))?;
                                    planned.insert(project);
                                    p.map(|mut p| { p.replaces = have.cloned(); p.dep = true; p })
                                }
                                _ => None,
                            };
                            match pick {
                                Some(p) if p.meta.as_ref().is_some_and(|m| m.mods.iter().any(|(i, v)| i == id && (v.is_empty() || crate::fit::satisfies(v, range, &m.kind)))) => {
                                    plan.push(p);
                                    next.push(plan.len() - 1);
                                }
                                _ => return Err(match &problem {
                                    Problem::Missing { .. } => format!("{}, and Obelisk couldn't find it on Modrinth for Minecraft {mc}. Add it first, then try again.", problem.describe(&name)),
                                    _ => format!("{}, and no newer version of it is available here. Update it first (or pick an older {name}).", problem.describe(&name)),
                                }),
                            }
                        }
                    }
                }
            }
            level = next;
            depth += 1;
        }
        if !soft.is_empty() && !req.force {
            soft.sort();
            soft.dedup();
            return Err(format!("{CONFIRM}{}", soft.join("\n")));
        }

        let dir = game.join(folder);
        fs::create_dir_all(dir.join(".index")).map_err(|e| e.to_string())?;
        let mut have = None;
        let mut done = Vec::new();
        for p in plan {
            if let Some(old) = &p.replaces {
                crate::delete_content(game, &old.kind, &old.file_name, |f| fs::remove_file(f).map_err(|e| e.to_string()))?;
            }
            let path = dir.join(&p.file.filename);
            fs::write(&path, &p.bytes).map_err(|e| e.to_string())?;
            if folder == "mods" {
                let ids = crate::dupes::own_ids(&path);
                let index = have.get_or_insert_with(|| crate::dupes::index(&dir));
                if let Some((_, other)) = crate::dupes::clash(index, &ids, &p.file.filename) {
                    let _ = fs::remove_file(&path);
                    if p.dep { continue }
                    return Err(format!("{} is already installed as {other} (probably from the other site); not adding a second copy", p.name));
                }
                for id in ids { index.entry(id).or_default().push(p.file.filename.clone()) }
            }
            fs::write(dir.join(".index").join(format!("{}.pw.toml", p.slug)), index_toml(&p.name, &p.v, &p.file)).map_err(|e| e.to_string())?;
            done.push(if p.replaces.is_some() && p.dep { format!("{} (updated)", p.name) } else { p.name });
        }
        Ok(done)
    }

    /// The newest version of a project that loads in this pack (releases first), downloaded. `Ok(None)` when the
    /// best one is `current` already. An exact `version` is checked too, and refused with the reason.
    fn choose(&self, ctx: &Ctx, project: &str, version: Option<&str>, current: Option<&str>, replacing: &[String]) -> Result<Option<Pick>, String> {
        let candidates = match version {
            Some(id) => vec![self.version(id)?],
            None => {
                let mut vs = self.versions(project, &loaders_for(ctx.project_type, ctx.loader), &[ctx.mc.to_string()], false)?;
                vs.sort_by_key(|v| v.version_type != "release"); // stable: newest first within each group
                vs.truncate(6);
                vs
            }
        };
        if candidates.is_empty() {
            return Err(format!("{project} has no version for Minecraft {} / {}", ctx.mc, ctx.loader));
        }
        let info = self.project(project)?;
        let name = info["title"].as_str().unwrap_or(project).to_string();
        let slug = safe_slug(info["slug"].as_str().unwrap_or(project));
        let mut why = Vec::new();
        for v in candidates {
            if current == Some(v.id.as_str()) {
                return Ok(None); // nothing newer loads here; keep what's installed
            }
            let file = primary_file(&v).ok_or("version has no files")?.clone();
            if file.filename.contains(['/', '\\']) || file.filename.contains("..") {
                return Err("bad file name from Modrinth".into());
            }
            let bytes = self.fetch_verified(&file)?;
            let meta = if ctx.checking { crate::fit::read_bundle(&bytes) } else { None };
            let refused = meta.as_ref().and_then(|m| crate::fit::check(m, ctx.pack, replacing).into_iter().find(|p| matches!(p, crate::fit::Problem::Unloadable(_))));
            match refused {
                Some(p) => why.push(p.describe(&format!("{name} {}", v.version_number))),
                None => return Ok(Some(Pick { v, file, bytes, name, slug, meta, replaces: None, dep: false })),
            }
        }
        Err(match why.len() {
            1 => why.remove(0),
            n => format!("{} (and the {} versions before it don't fit either)", why.remove(0), n - 1),
        })
    }

    /// The Modrinth project whose jar provides this mod id, found by name (YUNG's API is "yungsapi" in the jar).
    fn find_mod(&self, ctx: &Ctx, id: &str) -> Result<Option<String>, String> {
        let norm = |s: &str| s.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>();
        let want = norm(id);
        // Search matches words, and mod ids glue them together ("yungsapi"): try the id as written, with a common
        // library suffix split off, then by its first letters.
        let spaced = id.replace(['_', '-'], " ");
        let mut queries = vec![spaced.clone()];
        if let Some(stem) = ["api", "lib", "library", "core", "config", "mod"].iter().find_map(|s| spaced.strip_suffix(s).filter(|t| t.len() >= 3 && !t.ends_with(' '))) {
            queries.push(format!("{stem} {}", &spaced[stem.len()..]));
        }
        if want.len() > 5 { queries.push(want[..want.len().div_ceil(2).max(4)].to_string()) }
        for query in queries {
            let q = SearchQuery { query, project_type: "mod".into(), loaders: loaders_for("mod", ctx.loader),
                game_versions: vec![ctx.mc.to_string()], limit: 20, ..Default::default() };
            let hits = self.search(&q)?;
            let found = hits["hits"].as_array().into_iter().flatten()
                .find(|h| [h["slug"].as_str(), h["title"].as_str()].into_iter().flatten().any(|s| norm(s) == want))
                .and_then(|h| h["project_id"].as_str().map(str::to_string));
            if found.is_some() { return Ok(found) }
        }
        Ok(None)
    }

    fn fetch_verified(&self, f: &VersionFile) -> Result<Vec<u8>, String> {
        use sha2::{Digest, Sha512};
        let mut bytes = Vec::new();
        self.agent.get(&f.url).call().map_err(|e| format!("download: {e}"))?
            .into_reader().take(1 << 30).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if let Some(want) = f.hashes.get("sha512") {
            if &format!("{:x}", Sha512::digest(&bytes)) != want {
                return Err(format!("{}: hash mismatch", f.filename));
            }
        }
        Ok(bytes)
    }
}

/// Install errors that start with this need the player's yes; the request is sent again with `force`.
pub const CONFIRM: &str = "confirm: ";

struct Ctx<'a> {
    project_type: &'a str,
    loader: &'a str,
    mc: &'a str,
    pack: &'a crate::fit::Pack,
    checking: bool,
}

struct Pick {
    v: Version,
    file: VersionFile,
    bytes: Vec<u8>,
    name: String,
    slug: String,
    meta: Option<crate::fit::Meta>,
    replaces: Option<Installed>,
    dep: bool,
}

/// Does something already planned provide this mod id in a version the range accepts?
fn provided(plan: &[Pick], id: &str, range: &str, kind: &str) -> bool {
    plan.iter().filter_map(|p| p.meta.as_ref()).flat_map(|m| &m.mods)
        .any(|(i, v)| i == id && (v.is_empty() || crate::fit::satisfies(v, range, kind)))
}

/// The Modrinth project of the installed jar that provides this mod id, when Obelisk tracks it.
fn tracked_owner(game: &Path, folder: &str, existing: &[Installed], id: &str) -> Option<String> {
    let files = crate::dupes::index(&game.join(folder)).remove(id)?;
    existing.iter().find(|i| files.contains(&i.file_name)).map(|i| i.project_id.clone())
}

/// Modrinth project id of Fabric API.
pub const FABRIC_API: &str = "P7dR8mSH";

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct InstallRequest {
    pub project_id: String,
    pub project_type: String,
    /// Install this exact version instead of the best match.
    pub version_id: Option<String>,
    /// Use another loader's builds, e.g. "fabric" on NeoForge through Sinytra Connector.
    pub loader_override: Option<String>,
    /// The player said yes to the warnings (a mod that says it breaks with one in the pack).
    #[serde(default)]
    pub force: bool,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub project_id: String,
    pub version_id: String,
    pub kind: String,
    pub file_name: String,
}

fn deps_of(v: &Version) -> Vec<String> {
    v.dependencies.iter().filter(|d| d.dependency_type == "required").filter_map(|d| d.project_id.clone()).collect()
}

/// `.index` entries tracked by Modrinth: file name -> (project id, version id).
pub fn index_entries(dir: &Path) -> Vec<(String, (String, String))> {
    let Ok(entries) = fs::read_dir(dir) else { return vec![] };
    entries.flatten().filter_map(|e| {
        let doc = fs::read_to_string(e.path()).ok()?.parse::<toml::Table>().ok()?;
        let m = doc.get("update")?.get("modrinth")?;
        Some((doc.get("filename")?.as_str()?.to_string(),
              (m.get("mod-id")?.as_str()?.to_string(), m.get("version").and_then(|v| v.as_str()).unwrap_or_default().to_string())))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(id: &str, kind: &str) -> Version {
        serde_json::from_value(serde_json::json!({
            "id": id, "project_id": "P1", "name": id, "version_number": format!("{id}-num"),
            "version_type": kind, "loaders": ["neoforge"], "game_versions": ["1.21.1"],
            "files": [
                {"url": "https://cdn/x-sources.jar", "filename": "x-sources.jar", "hashes": {}, "primary": false},
                {"url": "https://cdn/x.jar", "filename": "x.jar", "hashes": {"sha512": "abc", "sha1": "def"}, "primary": true}
            ],
            "dependencies": [{"project_id": "DEP", "version_id": null, "dependency_type": "required"}]
        })).unwrap()
    }

    #[test]
    fn facets_and_each_category_in_its_own_group() {
        let q = SearchQuery {
            project_type: "mod".into(), loaders: vec!["neoforge".into()],
            game_versions: vec!["1.21.1".into()], categories: vec!["magic".into(), "storage".into()],
            ..Default::default()
        };
        assert_eq!(facets(&q),
            r#"[["project_type:mod"],["categories:neoforge"],["versions:1.21.1"],["categories:magic"],["categories:storage"]]"#);
        assert_eq!(facets(&SearchQuery::default()), "[]");
    }

    #[test]
    fn url_encoding() {
        assert_eq!(enc(r#"[["a:b"]] x"#), "%5B%5B%22a%3Ab%22%5D%5D%20x");
    }

    #[test]
    fn picks_newest_release_then_falls_back() {
        let vs = vec![version("beta2", "beta"), version("rel1", "release"), version("rel0", "release")];
        assert_eq!(pick_version(&vs).unwrap().id, "rel1");
        assert_eq!(pick_version(&vs[..1]).unwrap().id, "beta2");
        assert!(pick_version(&[]).is_none());
        assert_eq!(primary_file(&vs[0]).unwrap().filename, "x.jar");
    }

    #[test]
    fn loader_and_folder_mapping() {
        assert_eq!(loaders_for("mod", "NeoForge"), ["neoforge"]);
        assert_eq!(loaders_for("shader", "NeoForge"), ["iris", "optifine"]);
        assert_eq!(folder_for("resourcepack"), Some("resourcepacks"));
        assert_eq!(folder_for("modpack"), None);
    }

    #[test]
    fn index_toml_is_read_back_as_modrinth_tracked() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path();
        let v = version("V9", "release");
        let text = index_toml("Mod's Name", &v, primary_file(&v).unwrap());
        fs::create_dir_all(game.join("mods/.index")).unwrap();
        fs::write(game.join("mods/.index/x.pw.toml"), &text).unwrap();
        fs::write(game.join("mods/x.jar"), "jar").unwrap();

        assert!(installed_ids(game).contains("P1"));
        let items = crate::list_content(game, "mods");
        assert_eq!(items[0].source, "modrinth");
        assert_eq!(items[0].project_id, "P1");
        assert_eq!(items[0].name, "Mods Name");
        let doc: toml::Table = text.parse().unwrap();
        assert_eq!(doc["download"]["hash"].as_str(), Some("abc"));
        assert_eq!(doc["update"]["modrinth"]["version"].as_str(), Some("V9"));
    }

    #[test]
    #[ignore] // live network: cargo test -- --ignored
    fn live_search_and_versions() {
        let c = Client::new().unwrap();
        let q = SearchQuery { query: "sodium".into(), project_type: "mod".into(), ..Default::default() };
        let hits = c.search(&q).unwrap();
        assert!(hits["hits"].as_array().unwrap().len() > 0);
        let vs = c.versions("AANobbMI", &["neoforge".into()], &["1.21.1".into()], false).unwrap();
        assert!(pick_version(&vs).is_some());
        assert!(c.tags().unwrap()["gameVersions"].as_array().unwrap().iter().any(|v| v == "1.21.1"));
    }

    #[test]
    #[ignore] // live network: installs a mod with a required dependency into a temp folder
    fn live_install_with_dependency() {
        let tmp = tempfile::tempdir().unwrap();
        let c = Client::new().unwrap();
        // Waystones requires Balm on NeoForge 1.21.1.
        let req = InstallRequest { project_id: "waystones".into(), project_type: "mod".into(), ..Default::default() };
        let names = c.install(tmp.path(), &req, "NeoForge", "1.21.1", &[]).unwrap();
        assert!(names.len() >= 2, "expected a dependency too, got {names:?}");
        let items = crate::list_content(tmp.path(), "mods");
        assert_eq!(items.len(), names.len());
        assert!(items.iter().all(|i| i.source == "modrinth"));
        assert_eq!(installed_ids(tmp.path()).len(), names.len());

        // Installing an older version replaces the file instead of adding a second copy.
        let cache = tmp.path().join("cache");
        let existing = c.installed(tmp.path(), &cache).unwrap();
        assert_eq!(existing.len(), names.len());
        let all = c.versions("waystones", &["neoforge".into()], &["1.21.1".into()], false).unwrap();
        let older = all.iter().skip(1).find(|v| v.version_type == "release").unwrap();
        let req = InstallRequest { version_id: Some(older.id.clone()), ..req };
        c.install(tmp.path(), &req, "NeoForge", "1.21.1", &existing).unwrap();
        let now = c.installed(tmp.path(), &cache).unwrap();
        assert_eq!(now.len(), names.len());
        assert!(now.iter().any(|i| i.version_id == older.id));
    }

    #[test]
    #[ignore] // live network: a Fabric-only mod onto NeoForge through Connector skips Fabric API
    fn live_cross_loader_skips_fabric_api() {
        let tmp = tempfile::tempdir().unwrap();
        let c = Client::new().unwrap();
        // Mod Menu is Fabric-only and requires Fabric API.
        let req = InstallRequest { project_id: "modmenu".into(), project_type: "mod".into(),
                                   loader_override: Some("fabric".into()), ..Default::default() };
        let names = c.install(tmp.path(), &req, "NeoForge", "1.21.1", &[]).unwrap();
        assert_eq!(names[0], "Mod Menu");
        assert!(!names.iter().any(|n| n == "Fabric API"), "{names:?}");
    }

    #[test]
    #[ignore] // live network: CurseForge-sourced files are recognised by hash
    fn live_installed_by_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let c = Client::new().unwrap();
        let req = InstallRequest { project_id: "balm".into(), project_type: "mod".into(), ..Default::default() };
        c.install(tmp.path(), &req, "NeoForge", "1.21.1", &[]).unwrap();
        fs::remove_dir_all(tmp.path().join("mods/.index")).unwrap(); // now looks like a plain dropped-in jar
        let found = c.installed(tmp.path(), &tmp.path().join("cache")).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].project_id, "MBAkmtvl");
    }

    /// Real Modrinth: installs land on versions that load on an older NeoForge and bring the libraries the jars need.
    #[test]
    #[ignore]
    fn installs_fit_the_pack_end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("instance.cfg"), "name=Fit
").unwrap();
        fs::write(tmp.path().join("mmc-pack.json"), r#"{"components":[{"uid":"net.minecraft","version":"1.21.1"},{"uid":"net.neoforged","version":"21.1.252"}]}"#).unwrap();
        let game = tmp.path().join("minecraft");
        fs::create_dir_all(game.join("mods")).unwrap();
        let c = Client::new().unwrap();
        let req = |slug: &str| InstallRequest { project_id: slug.into(), project_type: "mod".into(), ..Default::default() };
        for slug in ["ecologics", "yungs-cave-biomes", "regions-unexplored", "structory-towers"] {
            let existing = c.installed(&game, &tmp.path().join("cache")).unwrap();
            let r = c.install(&game, &req(slug), "NeoForge", "1.21.1", &existing);
            eprintln!("{slug}: {r:?}");
            assert!(r.is_ok(), "{slug}: {r:?}");
        }
        let pack = crate::fit::pack(&game.join("mods"), "NeoForge", "21.1.252", "1.21.1");
        for f in fs::read_dir(game.join("mods")).unwrap().flatten().filter(|f| f.path().extension().is_some_and(|e| e == "jar")) {
            let meta = crate::fit::read_file(&f.path()).unwrap();
            let problems = crate::fit::check(&meta, &pack, &[]);
            eprintln!("{:?}: {problems:?}", f.file_name());
            assert!(problems.is_empty());
        }
        assert!(pack.mods.contains_key("yungsapi"));
    }
}
