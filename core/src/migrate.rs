//! Find profiles in other launchers and bring them into Prism as native instances.

use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceProfile {
    /// "curseforge"
    pub launcher: String,
    pub name: String,
    pub path: PathBuf,
    pub minecraft: String,
    /// Prism component uid of the loader, e.g. "net.neoforged" (empty for vanilla).
    pub loader_uid: String,
    pub loader_version: String,
    pub seconds_played: u64,
    pub last_played_ms: u64,
    /// Prism instance id if this profile was synced before.
    pub synced_as: Option<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Launcher {
    pub id: String,
    pub name: String,
    pub found: bool,
    /// Whether Obelisk can migrate from it yet.
    pub supported: bool,
    pub location: String,
}

fn env_path(var: &str) -> PathBuf {
    PathBuf::from(std::env::var(var).unwrap_or_default())
}

/// CurseForge's own list of Minecraft instances (Overwolf build).
pub fn curseforge_registry() -> PathBuf {
    env_path("LOCALAPPDATA").join("Overwolf").join("Curse").join("GameInstances").join("MinecraftGameInstance.json")
}

/// Which launchers are installed on this machine.
pub fn detect_launchers() -> Vec<Launcher> {
    let appdata = env_path("APPDATA");
    let home = env_path("USERPROFILE");
    let cf_registry = curseforge_registry();
    let cf_default = home.join("curseforge").join("minecraft").join("Instances");
    let cf = if cf_registry.is_file() { cf_registry } else { cf_default };
    let candidates: [(&str, &str, PathBuf, bool); 5] = [
        ("curseforge", "CurseForge", cf, true),
        ("modrinth", "Modrinth App", appdata.join("ModrinthApp").join("profiles"), false),
        ("multimc", "MultiMC", home.join("MultiMC").join("instances"), false),
        ("atlauncher", "ATLauncher", appdata.join("ATLauncher").join("instances"), false),
        ("gdlauncher", "GDLauncher", appdata.join("gdlauncher_carbon").join("data").join("instances"), false),
    ];
    candidates.into_iter().map(|(id, name, p, supported)| Launcher {
        id: id.into(), name: name.into(), found: p.exists(), supported, location: p.display().to_string(),
    }).collect()
}

/// Map CurseForge's loader string ("neoforge-21.1.252", "fabric-0.16.5-1.21.1") to a Prism component.
pub fn cf_loader(name: &str) -> (String, String) {
    let (kind, rest) = name.split_once('-').unwrap_or((name, ""));
    let version = match kind {
        "fabric" | "quilt" => rest.split('-').next().unwrap_or(rest),
        _ => rest,
    };
    let uid = match kind {
        "neoforge" => "net.neoforged",
        "forge" => "net.minecraftforge",
        "fabric" => "net.fabricmc.fabric-loader",
        "quilt" => "org.quiltmc.quilt-loader",
        _ => return (String::new(), String::new()),
    };
    (uid.into(), version.into())
}

fn read_json(path: &Path) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

/// CurseForge profiles: from its registry, plus any folder holding a minecraftinstance.json.
pub fn curseforge_profiles(registry: &Path, extra_dirs: &[PathBuf], prism_data: &Path) -> Vec<SourceProfile> {
    let mut paths: Vec<PathBuf> = read_json(registry)
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|i| i["installPath"].as_str().map(|p| PathBuf::from(p.trim_end_matches(['\\', '/']))))
        .collect();
    for dir in extra_dirs {
        if let Ok(entries) = fs::read_dir(dir) {
            paths.extend(entries.flatten().map(|e| e.path()).filter(|p| p.join("minecraftinstance.json").is_file()));
        }
    }
    let synced = synced_sources(prism_data);
    let mut seen = HashSet::new();
    paths.into_iter()
        .filter(|p| seen.insert(norm(p)))
        .filter_map(|p| {
            let v = read_json(&p.join("minecraftinstance.json"))?;
            let (loader_uid, loader_version) = cf_loader(v["baseModLoader"]["name"].as_str().unwrap_or_default());
            let last_played_ms = v["lastPlayed"].as_str()
                .and_then(parse_iso_ms).unwrap_or(0);
            Some(SourceProfile {
                launcher: "curseforge".into(),
                name: v["name"].as_str().unwrap_or("Unnamed").to_string(),
                minecraft: v["gameVersion"].as_str().unwrap_or_default().to_string(),
                loader_uid, loader_version,
                seconds_played: v["timePlayed"].as_u64().unwrap_or(0) / 1000,
                last_played_ms,
                synced_as: synced.get(&norm(&p)).cloned(),
                path: p,
            })
        })
        .collect()
}

fn norm(p: &Path) -> String {
    p.to_string_lossy().trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase()
}

/// Days-from-civil for "2026-10-19T21:55:26..." -> unix ms (UTC; good enough for "last played").
fn parse_iso_ms(s: &str) -> Option<u64> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>());
    let (y, m, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    let mut t = time.split([':', '.', 'Z', '+']).map(|x| x.parse::<i64>().unwrap_or(0));
    let (hh, mm, ss) = (t.next()?, t.next().unwrap_or(0), t.next().unwrap_or(0));
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(((days * 86400 + hh * 3600 + mm * 60 + ss) * 1000) as u64)
}

/// Source folder -> Prism instance id, for instances Obelisk created.
pub fn synced_sources(prism_data: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(entries) = fs::read_dir(crate::instances_dir(prism_data)) else { return map };
    for e in entries.flatten() {
        let cfg = crate::read_cfg(&e.path().join("instance.cfg"));
        if let Some(src) = cfg.get("ObeliskSource").filter(|s| !s.is_empty()) {
            map.insert(norm(Path::new(src)), e.file_name().to_string_lossy().into_owned());
        }
    }
    map
}

/// Folders/files in a CurseForge instance that are caches, logs or launcher bookkeeping.
fn skip(rel: &Path) -> bool {
    let first = rel.components().next().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).unwrap_or_default();
    let name = rel.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let top_dirs = [".cache", "cache", "logs", "crash-reports", ".mixin.out", ".stfolder", "debug", "mod-backups", "downloads"];
    let top_files = ["minecraftinstance.json", ".curseclient", ".stignore", "usercache.json"];
    top_dirs.contains(&first.as_str())
        || (rel.components().count() == 1 && (top_files.contains(&name.as_str()) || name.starts_with("hs_err_pid") || name.ends_with(".log")))
}

/// Copy `src` into `dst`. `only` limits the copy to those top-level folders; files are copied
/// only when missing or changed (size or mtime). Returns bytes copied.
pub fn mirror(src: &Path, dst: &Path, only: Option<&[&str]>, remove_extra: bool, progress: &dyn Fn(u64)) -> Result<u64, String> {
    let mut copied = 0u64;
    let mut stack = vec![PathBuf::new()];
    let mut kept: HashSet<PathBuf> = HashSet::new();
    while let Some(rel) = stack.pop() {
        let Ok(entries) = fs::read_dir(src.join(&rel)) else { continue };
        for e in entries.flatten() {
            let r = rel.join(e.file_name());
            if let Some(only) = only {
                let top = r.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
                if !only.contains(&top.as_str()) {
                    continue;
                }
            }
            if skip(&r) {
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                if !(r.components().count() == 1 && r.to_string_lossy().starts_with("mods-removed-")) {
                    fs::create_dir_all(dst.join(&r)).map_err(|e| e.to_string())?;
                    kept.insert(r.clone());
                    stack.push(r);
                }
                continue;
            }
            kept.insert(r.clone());
            let target = dst.join(&r);
            let same = fs::metadata(&target).ok().is_some_and(|t| t.len() == meta.len() && t.modified().ok() >= meta.modified().ok());
            if !same {
                fs::copy(e.path(), &target).map_err(|err| format!("{}: {err}", r.display()))?;
                copied += meta.len();
                progress(copied);
            }
        }
    }
    if remove_extra {
        for top in only.unwrap_or(&[]) {
            remove_unkept(&dst.join(top), Path::new(top), &kept)?;
        }
    }
    Ok(copied)
}

fn remove_unkept(dir: &Path, rel: &Path, kept: &HashSet<PathBuf>) -> Result<(), String> {
    let Ok(entries) = fs::read_dir(dir) else { return Ok(()) };
    for e in entries.flatten() {
        let r = rel.join(e.file_name());
        if r.components().any(|c| c.as_os_str() == ".index") {
            continue; // metadata is regenerated separately
        }
        if e.path().is_dir() {
            if kept.contains(&r) { remove_unkept(&e.path(), &r, kept)?; } else { fs::remove_dir_all(e.path()).map_err(|e| e.to_string())?; }
        } else if !kept.contains(&r) {
            fs::remove_file(e.path()).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

const CLASS_DIRS: [(u64, &str); 4] = [(6, "mods"), (12, "resourcepacks"), (6552, "shaderpacks"), (6945, "datapacks")];

/// Prism `.index` entries for every CurseForge-tracked file, so Prism can update them.
pub fn write_curseforge_index(src_manifest: &Value, game: &Path) -> Result<usize, String> {
    let mut n = 0;
    for kind in crate::KINDS {
        let dir = game.join(kind).join(".index");
        if let Ok(entries) = fs::read_dir(&dir) {
            for e in entries.flatten() {
                let text = fs::read_to_string(e.path()).unwrap_or_default();
                if text.contains("[update.curseforge]") {
                    let _ = fs::remove_file(e.path()); // regenerated below
                }
            }
        }
    }
    for a in src_manifest["installedAddons"].as_array().into_iter().flatten() {
        let f = &a["installedFile"];
        let folder = CLASS_DIRS.iter().find(|(id, _)| Some(*id) == a["categoryClassID"].as_u64()).map(|x| x.1).unwrap_or("mods");
        if folder == "datapacks" {
            continue;
        }
        let file = f["fileName"].as_str().unwrap_or_default();
        if file.is_empty() || file.contains(['/', '\\']) {
            continue;
        }
        let sha1 = f["hashes"].as_array().into_iter().flatten()
            .find(|h| h["type"].as_u64() == Some(1)).and_then(|h| h["value"].as_str()).unwrap_or_default().to_lowercase();
        let slug = a["webSiteURL"].as_str().and_then(|u| u.trim_end_matches('/').rsplit('/').next())
            .map(|s| s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect::<String>())
            .unwrap_or_else(|| a["addonID"].to_string());
        let q = |s: &str| format!("'{}'", s.replace('\'', ""));
        let text = format!(
            "filename = {}\nname = {}\nside = ''\n\n[download]\nhash = {}\nhash-format = 'sha1'\nmode = 'metadata:curseforge'\nurl = ''\n\n[update.curseforge]\nfile-id = {}\nproject-id = {}\n",
            q(file), q(a["name"].as_str().unwrap_or(file).trim()), q(&sha1),
            f["id"].as_u64().unwrap_or(0), a["addonID"].as_u64().unwrap_or(0));
        let dir = game.join(folder).join(".index");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        fs::write(dir.join(format!("{slug}.pw.toml")), text).map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

fn unique_id(instances: &Path, name: &str) -> String {
    let base: String = name.chars().map(|c| if c.is_alphanumeric() || " -_.()".contains(c) { c } else { '_' }).collect();
    let base = base.trim().trim_end_matches('.').to_string();
    let base = if base.is_empty() { "Imported".to_string() } else { base };
    (1..).map(|i| if i == 1 { base.clone() } else { format!("{base} ({i})") })
        .find(|id| !instances.join(id).exists()).unwrap()
}

/// Create a Prism instance from a CurseForge profile, or refresh one synced before
/// (mods, packs and configs only; worlds are never overwritten). Returns the instance id.
pub fn sync_curseforge(p: &SourceProfile, prism_data: &Path, progress: &dyn Fn(u64)) -> Result<String, String> {
    let manifest = read_json(&p.path.join("minecraftinstance.json")).ok_or("minecraftinstance.json missing")?;
    let instances = crate::instances_dir(prism_data);
    let (id, fresh) = match &p.synced_as {
        Some(id) if instances.join(id).is_dir() => (id.clone(), false),
        _ => (unique_id(&instances, &p.name), true),
    };
    let root = instances.join(&id);
    let game = root.join("minecraft");
    fs::create_dir_all(&game).map_err(|e| e.to_string())?;

    if fresh {
        mirror(&p.path, &game, None, false, progress)?;
    } else {
        mirror(&p.path, &game, Some(&["mods", "resourcepacks", "shaderpacks", "config", "defaultconfigs", "kubejs"]), true, progress)?;
    }
    write_curseforge_index(&manifest, &game)?;

    let mut components = vec![serde_json::json!({ "uid": "net.minecraft", "version": p.minecraft, "important": true })];
    if !p.loader_uid.is_empty() {
        components.push(serde_json::json!({ "uid": p.loader_uid, "version": p.loader_version }));
    }
    fs::write(root.join("mmc-pack.json"), serde_json::to_string_pretty(&serde_json::json!({ "formatVersion": 1, "components": components })).unwrap())
        .map_err(|e| e.to_string())?;

    let mut set: Vec<(&str, String)> = vec![
        ("InstanceType", "OneSix".into()),
        ("ObeliskSource", p.path.display().to_string()),
        ("totalTimePlayed", p.seconds_played.to_string()),
        ("lastLaunchTime", p.last_played_ms.to_string()),
    ];
    if fresh {
        set.push(("name", p.name.clone()));
        if let Some(icon) = manifest["profileImagePath"].as_str().map(PathBuf::from).filter(|i| i.is_file()) {
            let key = format!("obelisk-{}", id.to_lowercase().replace(|c: char| !c.is_ascii_alphanumeric(), "-"));
            let icons = crate::icons_dir(prism_data);
            fs::create_dir_all(&icons).map_err(|e| e.to_string())?;
            let ext = icon.extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
            fs::copy(&icon, icons.join(format!("{key}.{ext}"))).map_err(|e| e.to_string())?;
            set.push(("iconKey", key));
        }
        if manifest["isMemoryOverride"].as_bool() == Some(true) {
            if let Some(mb) = manifest["allocatedMemory"].as_u64() {
                set.push(("OverrideMemory", "true".into()));
                set.push(("MaxMemAlloc", mb.to_string()));
            }
        }
        if let Some(args) = manifest["javaArgsOverride"].as_str().filter(|a| !a.trim().is_empty()) {
            set.push(("OverrideJavaArgs", "true".into()));
            set.push(("JvmArgs", args.trim().to_string()));
        }
    }
    let pairs: Vec<(&str, Option<String>)> = set.into_iter().map(|(k, v)| (k, Some(v))).collect();
    crate::update_cfg(&root.join("instance.cfg"), &pairs)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    fn cf_instance(dir: &Path) {
        write(&dir.join("minecraftinstance.json"), &serde_json::json!({
            "name": "Foundation", "gameVersion": "1.21.1", "timePlayed": 7_200_000u64,
            "lastPlayed": "2026-10-19T21:55:26.0Z",
            "baseModLoader": { "name": "neoforge-21.1.252" },
            "isMemoryOverride": true, "allocatedMemory": 6000, "javaArgsOverride": "-XX:+UseZGC",
            "profileImagePath": dir.join("Foundation.png"),
            "installedAddons": [
                { "addonID": 42, "name": "Jade", "categoryClassID": 6, "webSiteURL": "https://www.curseforge.com/minecraft/mc-mods/jade",
                  "installedFile": { "id": 9001, "fileName": "jade.jar", "hashes": [{ "type": 1, "value": "ABC" }] } },
                { "addonID": 43, "name": "Pack", "categoryClassID": 12, "webSiteURL": "https://www.curseforge.com/minecraft/texture-packs/pack",
                  "installedFile": { "id": 9002, "fileName": "pack.zip", "hashes": [] } }
            ]
        }).to_string());
        write(&dir.join("Foundation.png"), "png");
        write(&dir.join("mods/jade.jar"), "jar");
        write(&dir.join("mods/mydev.jar"), "dev");
        write(&dir.join("resourcepacks/pack.zip"), "zip");
        write(&dir.join("config/jade.toml"), "x=1");
        write(&dir.join("saves/World/level.dat"), "world");
        write(&dir.join("logs/latest.log"), "log");
        write(&dir.join(".stfolder/x"), "sync");
        write(&dir.join("hs_err_pid1.log"), "crash");
    }

    #[test]
    fn loader_strings() {
        assert_eq!(cf_loader("neoforge-21.1.252"), ("net.neoforged".into(), "21.1.252".into()));
        assert_eq!(cf_loader("forge-47.2.0"), ("net.minecraftforge".into(), "47.2.0".into()));
        assert_eq!(cf_loader("fabric-0.16.5-1.21.1"), ("net.fabricmc.fabric-loader".into(), "0.16.5".into()));
        assert_eq!(cf_loader(""), ("".into(), "".into()));
        assert_eq!(parse_iso_ms("1970-01-02T00:00:01Z"), Some(86_401_000));
    }

    #[test]
    fn finds_profiles_via_registry_and_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let cf = tmp.path().join("cf/Instances/Foundation");
        cf_instance(&cf);
        let registry = tmp.path().join("registry.json");
        write(&registry, &serde_json::json!([{ "installPath": format!("{}\\", cf.display()) }]).to_string());
        let prism = tmp.path().join("prism");
        // Found once even though both the registry and the folder scan see it.
        let list = curseforge_profiles(&registry, &[tmp.path().join("cf/Instances")], &prism);
        assert_eq!(list.len(), 1);
        let p = &list[0];
        assert_eq!((p.name.as_str(), p.minecraft.as_str(), p.loader_uid.as_str()), ("Foundation", "1.21.1", "net.neoforged"));
        assert_eq!(p.seconds_played, 7200);
        assert!(p.synced_as.is_none());
    }

    #[test]
    fn sync_creates_a_prism_instance_then_resyncs_without_touching_worlds() {
        let tmp = tempfile::tempdir().unwrap();
        let cf = tmp.path().join("cf/Foundation");
        cf_instance(&cf);
        let prism = tmp.path().join("prism");
        let profiles = curseforge_profiles(Path::new("missing"), &[tmp.path().join("cf")], &prism);
        let id = sync_curseforge(&profiles[0], &prism, &|_| {}).unwrap();
        assert_eq!(id, "Foundation");

        let root = prism.join("instances/Foundation");
        let game = root.join("minecraft");
        for f in ["mods/jade.jar", "mods/mydev.jar", "resourcepacks/pack.zip", "config/jade.toml", "saves/World/level.dat"] {
            assert!(game.join(f).is_file(), "{f} missing");
        }
        for f in ["logs", ".stfolder", "hs_err_pid1.log", "minecraftinstance.json"] {
            assert!(!game.join(f).exists(), "{f} should be skipped");
        }
        let inst = &crate::list_instances(&prism)[0];
        assert_eq!((inst.name.as_str(), inst.loader.as_str(), inst.loader_version.as_str()), ("Foundation", "NeoForge", "21.1.252"));
        assert_eq!(inst.seconds_played, 7200);
        let cfg = crate::read_cfg(&root.join("instance.cfg"));
        assert_eq!(cfg["MaxMemAlloc"], "6000");
        assert_eq!(cfg["JvmArgs"], "-XX:+UseZGC");
        assert!(prism.join(format!("icons/{}.png", cfg["iconKey"])).is_file());
        let items = crate::list_content(&game, "mods");
        let jade = items.iter().find(|i| i.file_name == "jade.jar").unwrap();
        assert_eq!((jade.source.as_str(), jade.project_id.as_str()), ("curseforge", "42"));
        assert_eq!(items.iter().find(|i| i.file_name == "mydev.jar").unwrap().source, "local");

        // Second pass: profile is recognised, mod removals mirror, worlds stay as played in Prism.
        fs::remove_file(cf.join("mods/mydev.jar")).unwrap();
        fs::write(game.join("saves/World/level.dat"), "played in prism").unwrap();
        let profiles = curseforge_profiles(Path::new("missing"), &[tmp.path().join("cf")], &prism);
        assert_eq!(profiles[0].synced_as.as_deref(), Some("Foundation"));
        assert_eq!(sync_curseforge(&profiles[0], &prism, &|_| {}).unwrap(), "Foundation");
        assert!(!game.join("mods/mydev.jar").exists());
        assert!(game.join("mods/.index/jade.pw.toml").is_file());
        assert_eq!(fs::read_to_string(game.join("saves/World/level.dat")).unwrap(), "played in prism");
        assert_eq!(crate::list_instances(&prism).len(), 1);
    }
}
