#![recursion_limit = "256"]
use obelisk_core as prism;
mod bridge;
mod cleanup;
mod control;
mod cfaccount;
mod errlog;
mod shot;
mod releases;
mod watch;
mod datadir;
mod devrun;
mod mcp;
mod selfupdate;

/// The build stamp (`obelisk.exe --build-id`, used when publishing an update).
pub fn build_id() -> &'static str {
    selfupdate::BUILD
}

/// `obelisk.exe --mcp`: serve AI assistants over stdio (see mcp.rs).
pub fn mcp_serve() {
    mcp::serve()
}

/// `obelisk.exe --reset-data` (the installer's clean install) and `--uninstall-cleanup` (the uninstaller); see cleanup.rs.
pub fn reset_data() {
    cleanup::reset_data()
}

pub fn uninstall_cleanup(keep_data: bool) -> Result<(), String> {
    cleanup::uninstall(keep_data)
}

use prism::{ContentItem, Instance, World};
use std::collections::HashMap;
use std::path::PathBuf;
use std::os::windows::ffi::OsStrExt;
use tauri::Manager;

fn game(instance_id: &str) -> Result<PathBuf, String> {
    prism::game_dir(&prism::data_dir(), instance_id).ok_or_else(|| format!("instance {instance_id} not found"))
}

fn kind_ok(kind: &str) -> Result<(), String> {
    if prism::KINDS.contains(&kind) { Ok(()) } else { Err(format!("unknown content kind {kind}")) }
}

// Commands are async so disk work runs off the UI thread.
#[tauri::command]
async fn list_instances() -> Vec<Instance> {
    prism::list_instances(&prism::data_dir())
}

#[tauri::command]
async fn instance_icon(icon_key: String) -> Option<String> {
    prism::icon_data_url(&prism::data_dir(), &icon_key)
}

/// Library → Change Picture: `png` is the picture the window already cropped and shrank (base64).
#[tauri::command]
async fn set_instance_icon(instance_id: String, png: String) -> Result<String, String> {
    blocking(move || prism::set_icon_base64(&prism::data_dir(), &instance_id, &png)).await
}

#[tauri::command]
async fn list_content(instance_id: String, kind: String) -> Result<Vec<ContentItem>, String> {
    kind_ok(&kind)?;
    Ok(prism::list_content(&game(&instance_id)?, &kind))
}

/// File name -> extracted icon path (served to the UI over the asset protocol).
#[tauri::command]
async fn content_icons(_app: tauri::AppHandle, instance_id: String, kind: String) -> Result<HashMap<String, PathBuf>, String> {
    kind_ok(&kind)?;
    let cache = datadir::root().join("icons");
    Ok(prism::content_icons(&game(&instance_id)?, &kind, &cache))
}

#[tauri::command]
async fn set_enabled(instance_id: String, kind: String, file_name: String, enabled: bool) -> Result<String, String> {
    prism::set_enabled(&game(&instance_id)?, &kind, &file_name, enabled)
}

/// Sends the file (and its Prism metadata) to the recycle bin.
#[tauri::command]
async fn delete_content(instance_id: String, kind: String, file_name: String, name: String, version: String, authors: String, source: String) -> Result<(), String> {
    prism::trash::put(&game(&instance_id)?, &kind, &file_name, &name, &version, &authors, &source).map(|_| ())
}

/// How long deleted content stays in a profile's trash (Settings → General), default a week.
fn trash_ms(app: &tauri::AppHandle) -> u64 {
    let hours = settings_value(app).get("trashHours").and_then(|v| v.as_u64()).unwrap_or(336);
    hours.max(1) * 3_600_000
}

fn pack_trash_ms(app: &tauri::AppHandle) -> u64 {
    let hours = settings_value(app).get("packTrashHours").and_then(|v| v.as_u64()).unwrap_or(336);
    hours.max(1) * 3_600_000
}

/// How many automatic backups and checkpoints to keep (Settings > Backups).
fn backup_limits(app: &tauri::AppHandle) -> (usize, usize) {
    let s = settings_value(app);
    let b = s.get("backups");
    let n = |k: &str, d: u64| b.and_then(|b| b.get(k)).and_then(|v| v.as_u64()).unwrap_or(d).max(1) as usize;
    (n("keepAuto", 10), n("keepManual", 20))
}

#[tauri::command]
async fn list_trash(app: tauri::AppHandle, instance_id: String) -> Result<Vec<prism::trash::TrashEntry>, String> {
    let game = game(&instance_id)?;
    prism::trash::purge(&game, trash_ms(&app));
    Ok(prism::trash::list(&game))
}

#[tauri::command]
async fn restore_trash(instance_id: String, id: String) -> Result<prism::trash::TrashEntry, String> {
    prism::trash::restore(&game(&instance_id)?, &id)
}

#[tauri::command]
async fn remove_trash(instance_id: String, id: String) -> Result<(), String> {
    prism::trash::remove(&game(&instance_id)?, &id)
}

#[tauri::command]
async fn list_worlds(instance_id: String) -> Result<Vec<World>, String> {
    Ok(prism::list_worlds(&game(&instance_id)?))
}

#[tauri::command]
async fn list_screenshots(instance_id: String) -> Result<Vec<PathBuf>, String> {
    Ok(prism::list_screenshots(&game(&instance_id)?))
}

#[tauri::command]
async fn list_logs(instance_id: String) -> Result<Vec<String>, String> {
    Ok(prism::list_logs(&game(&instance_id)?))
}

#[tauri::command]
async fn read_log(instance_id: String, name: String) -> Result<String, String> {
    prism::read_log_tail(&game(&instance_id)?, &name, 512 * 1024)
}

/// Open a file or folder of an instance in Explorer.
#[tauri::command]
async fn reveal(instance_id: String, sub: String) -> Result<(), String> {
    if sub.contains("..") {
        return Err("invalid path".into());
    }
    let path = game(&instance_id)?.join(sub).to_string_lossy().replace('/', "\\");
    std::process::Command::new("explorer").arg(&path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

// ---- Modrinth (blocking HTTP runs on a worker thread so the UI never stalls) ----

type Mr = std::sync::Arc<prism::modrinth::Client>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn mr_search(mr: tauri::State<'_, Mr>, query: prism::modrinth::SearchQuery) -> Result<serde_json::Value, String> {
    let mr = mr.inner().clone();
    blocking(move || mr.search(&query)).await
}

#[tauri::command]
async fn mr_tags(mr: tauri::State<'_, Mr>) -> Result<serde_json::Value, String> {
    let mr = mr.inner().clone();
    blocking(move || mr.tags()).await
}

#[tauri::command]
async fn mr_project(mr: tauri::State<'_, Mr>, id: String) -> Result<serde_json::Value, String> {
    let mr = mr.inner().clone();
    blocking(move || mr.project(&id)).await
}

#[tauri::command]
async fn mr_versions(mr: tauri::State<'_, Mr>, id: String, changelog: bool) -> Result<Vec<prism::modrinth::Version>, String> {
    let mr = mr.inner().clone();
    blocking(move || mr.versions(&id, &[], &[], changelog)).await
}

#[tauri::command]
async fn active_account() -> Option<prism::Account> {
    prism::active_account(&prism::data_dir())
}

#[tauri::command]
async fn mr_user(mr: tauri::State<'_, Mr>, name: String) -> Result<serde_json::Value, String> {
    let mr = mr.inner().clone();
    blocking(move || Ok(serde_json::json!({ "user": mr.user(&name)?, "projects": mr.user_projects(&name)? }))).await
}

#[tauri::command]
async fn mr_members(mr: tauri::State<'_, Mr>, id: String) -> Result<serde_json::Value, String> {
    let mr = mr.inner().clone();
    blocking(move || mr.members(&id)).await
}

#[tauri::command]
async fn mr_dependencies(mr: tauri::State<'_, Mr>, id: String) -> Result<serde_json::Value, String> {
    let mr = mr.inner().clone();
    blocking(move || mr.dependencies(&id)).await
}

/// Project page → Relations for a CurseForge project (see prism::curseforge::relations).
#[tauri::command]
async fn cf_relations(id: u64, which: String, index: u32, kind: String, sort: Option<String>) -> Result<serde_json::Value, String> {
    blocking(move || prism::curseforge::relations(id, &which, index, &kind, sort.as_deref().unwrap_or("popular"))).await
}

// ---- Modrinth account: a personal access token in Windows Credential Manager, never sent back to the UI ----

fn mr_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new("Obelisk", "modrinth-token").map_err(|e| e.to_string())
}

fn mr_token() -> Result<String, String> {
    mr_entry()?.get_password().ok().filter(|t| !t.is_empty())
        .ok_or_else(|| "Sign in to Modrinth in Settings → Accounts first.".into())
}

/// Save the token if Modrinth accepts it (empty signs out). Returns the account.
#[tauri::command]
async fn mr_token_set(mr: tauri::State<'_, Mr>, token: String) -> Result<serde_json::Value, String> {
    let token = token.trim().to_string();
    if token.is_empty() {
        mr_entry()?.delete_credential().or(Ok::<(), keyring::Error>(())).map_err(|e| e.to_string())?;
        return Ok(serde_json::Value::Null);
    }
    let mr = mr.inner().clone();
    let t = token.clone();
    let me = blocking(move || mr.me(&t)).await?;
    mr_entry()?.set_password(&token).map_err(|e| e.to_string())?;
    Ok(me)
}

/// The signed-in Modrinth account (null when signed out).
#[tauri::command]
async fn mr_account(mr: tauri::State<'_, Mr>) -> Result<serde_json::Value, String> {
    let Ok(token) = mr_token() else { return Ok(serde_json::Value::Null) };
    let mr = mr.inner().clone();
    blocking(move || mr.me(&token)).await
}

/// Ids of the projects the signed-in account follows.
#[tauri::command]
async fn mr_follows(mr: tauri::State<'_, Mr>) -> Result<Vec<String>, String> {
    let token = mr_token()?;
    let mr = mr.inner().clone();
    blocking(move || {
        let me = mr.me(&token)?;
        let list = mr.follows(&token, me["id"].as_str().unwrap_or_default())?;
        Ok(list.as_array().into_iter().flatten().filter_map(|p| p["id"].as_str().map(String::from)).collect())
    }).await
}

#[tauri::command]
async fn mr_follow(mr: tauri::State<'_, Mr>, id: String, on: bool) -> Result<(), String> {
    let token = mr_token()?;
    let mr = mr.inner().clone();
    blocking(move || mr.follow(&token, &id, on)).await
}

#[tauri::command]
async fn mr_notifications(mr: tauri::State<'_, Mr>) -> Result<serde_json::Value, String> {
    let token = mr_token()?;
    let mr = mr.inner().clone();
    blocking(move || {
        let me = mr.me(&token)?;
        mr.notifications(&token, me["id"].as_str().unwrap_or_default())
    }).await
}

#[tauri::command]
async fn mr_notifications_read(mr: tauri::State<'_, Mr>, ids: Vec<String>) -> Result<(), String> {
    let token = mr_token()?;
    let mr = mr.inner().clone();
    blocking(move || mr.mark_read(&token, &ids)).await
}

fn hash_cache(_app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(datadir::root().join("hashes"))
}

/// Modrinth projects present in an instance, by `.index` metadata or by exact file hash.
#[tauri::command]
async fn mr_installed(app: tauri::AppHandle, mr: tauri::State<'_, Mr>, instance_id: String) -> Result<Vec<prism::modrinth::Installed>, String> {
    let mr = mr.inner().clone();
    let (game, cache) = (game(&instance_id)?, hash_cache(&app)?);
    blocking(move || mr.installed(&game, &cache)).await
}

/// Install (or switch the version of) a mod/resource pack/shader plus its required dependencies.
#[tauri::command]
async fn mr_install(app: tauri::AppHandle, mr: tauri::State<'_, Mr>, instance_id: String, req: prism::modrinth::InstallRequest, world: Option<String>) -> Result<Vec<String>, String> {
    let mr = mr.inner().clone();
    let inst = prism::list_instances(&prism::data_dir()).into_iter().find(|i| i.id == instance_id)
        .ok_or("instance not found")?;
    let (game, cache) = (game(&instance_id)?, hash_cache(&app)?);
    blocking(move || {
        // Data packs go into one world's datapacks/ folder; that world is the install root.
        if let Some(world) = world {
            let root = prism::world_dir(&game, &world)?;
            let existing: Vec<_> = prism::modrinth::index_entries(&root.join("datapacks").join(".index")).into_iter()
                .filter(|(base, _)| root.join("datapacks").join(base).exists())
                .map(|(base, (project_id, version_id))| prism::modrinth::Installed { project_id, version_id, kind: "datapacks".into(), file_name: base })
                .collect();
            return mr.install(&root, &req, &inst.loader, &inst.minecraft, &existing);
        }
        let existing = mr.installed(&game, &cache)?;
        mr.install(&game, &req, &inst.loader, &inst.minecraft, &existing)
    }).await
}

// ---- Sync from other launchers ----

#[tauri::command]
async fn detect_launchers() -> Vec<prism::migrate::Launcher> {
    prism::migrate::detect_launchers()
}

#[tauri::command]
async fn sync_sources(extra_dirs: Vec<PathBuf>) -> Vec<prism::migrate::SourceProfile> {
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let mut dirs = extra_dirs;
    dirs.push(home.join("curseforge").join("minecraft").join("Instances"));
    prism::migrate::curseforge_profiles(&prism::migrate::curseforge_registry(), &dirs, &prism::data_dir())
}

/// Bring one profile over (or refresh it). Emits `sync-progress` {path, bytes} while copying.
#[tauri::command]
async fn sync_profile(app: tauri::AppHandle, path: PathBuf, extra_dirs: Vec<PathBuf>) -> Result<String, String> {
    use tauri::Emitter;
    let profile = sync_sources(extra_dirs).await.into_iter().find(|p| p.path == path).ok_or("profile not found")?;
    blocking(move || {
        let last = std::sync::atomic::AtomicU64::new(0);
        prism::migrate::sync_curseforge(&profile, &prism::data_dir(), &|bytes| {
            // throttle to ~every 32 MB
            if bytes - last.load(std::sync::atomic::Ordering::Relaxed) > 32 << 20 {
                last.store(bytes, std::sync::atomic::Ordering::Relaxed);
                let _ = app.emit("sync-progress", serde_json::json!({ "path": profile.path, "bytes": bytes }));
            }
        })
    }).await
}

// ---- Profiles, Prism defaults, app settings ----

#[tauri::command]
async fn profile_options(instance_id: String) -> Result<prism::ProfileOptions, String> {
    prism::profile_options(&prism::data_dir(), &instance_id)
}

/// Java installs found on this PC (Prism's downloads and the usual vendor folders).
#[tauri::command]
async fn java_installs() -> Vec<prism::java::JavaInstall> {
    prism::java::installs(&prism::data_dir())
}

/// Install Java `major` (Eclipse Temurin) into Prism's java folder. Progress goes out as
/// "java-progress" events ({major, done, total} in bytes).
#[tauri::command]
async fn java_download(app: tauri::AppHandle, major: u32) -> Result<prism::java::JavaInstall, String> {
    if ![8, 11, 17, 21, 25].contains(&major) {
        return Err(format!("Java {major} isn't offered"));
    }
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Emitter;
        let dest = prism::java::download_dir(&prism::data_dir(), major);
        let mut last = 0;
        prism::java::install(major, &dest, |done, total| {
            if done - last >= 1 << 20 || done == total {
                last = done;
                let _ = app.emit("java-progress", serde_json::json!({ "major": major, "done": done, "total": total }));
            }
        })
    }).await.map_err(|e| e.to_string())?
}

/// The Java at a path the user picked (its java/javaw.exe or its folder), if it is one.
#[tauri::command]
fn java_at(path: String) -> Option<prism::java::JavaInstall> {
    prism::java::at(std::path::Path::new(&path))
}

#[tauri::command]
async fn set_profile_options(instance_id: String, options: prism::ProfileOptions) -> Result<(), String> {
    prism::set_profile_options(&prism::data_dir(), &instance_id, &options)
}

#[tauri::command]
async fn clone_instance(instance_id: String, name: String) -> Result<String, String> {
    blocking(move || prism::work::run("Copying the pack", || prism::clone_instance(&prism::data_dir(), &instance_id, &name))).await
}

#[tauri::command]
async fn delete_instance(instance_id: String) -> Result<(), String> {
    blocking(move || prism::delete_instance(&prism::data_dir(), &instance_id).map(|_| ())).await
}

#[tauri::command]
async fn pack_trash(app: tauri::AppHandle) -> Result<Vec<prism::packtrash::TrashedPack>, String> {
    let dir = prism::instances_dir(&prism::data_dir());
    let max = pack_trash_ms(&app);
    blocking(move || {
        prism::packtrash::purge(&dir, max, |p| trash::delete(p).map_err(|e| e.to_string()));
        Ok(prism::packtrash::list(&dir))
    }).await
}

#[tauri::command]
async fn restore_pack(id: String) -> Result<String, String> {
    blocking(move || prism::packtrash::restore(&prism::instances_dir(&prism::data_dir()), &id)).await
}

/// "Delete forever" from the modpack trash (still goes through the Recycle Bin).
#[tauri::command]
async fn remove_pack(id: String) -> Result<(), String> {
    blocking(move || prism::packtrash::remove(&prism::instances_dir(&prism::data_dir()), &id, |p| trash::delete(p).map_err(|e| e.to_string()))).await
}

// ---- shared packs (peer to peer; see core/src/share.rs and share_net.rs)

type ShareNet = std::sync::Arc<tokio::sync::OnceCell<prism::share_net::Net>>;

/// The peer, started the first time it's needed. Our identity key is kept in the app's data folder.
async fn share_net<'a>(_app: &tauri::AppHandle, cell: &'a ShareNet) -> Result<&'a prism::share_net::Net, String> {
    let dir = datadir::root();
    cell.get_or_try_init(|| async move {
        let key_file = dir.join("share-key");
        let key: [u8; 32] = match std::fs::read(&key_file).ok().and_then(|b| b.try_into().ok()) {
            Some(k) => k,
            None => {
                let k = iroh_key();
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                std::fs::write(&key_file, k).map_err(|e| e.to_string())?;
                k
            }
        };
        let name = prism::active_account(&prism::data_dir()).map(|a| a.name).unwrap_or_else(|| "Player".into());
        prism::share_net::Net::start(key, prism::instances_dir(&prism::data_dir()), name).await
    }).await
}

fn iroh_key() -> [u8; 32] {
    prism::share_net::new_key()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ShareStatus {
    shared: bool,
    me: String,
    /// The pack's owner (who can remove people).
    owner: String,
    /// Set when the owner removed us: who and when (the pack's page says so until dismissed).
    ended: Option<prism::share::Ended>,
    share: Option<prism::share::Share>,
    /// Friends' changes waiting, each with `jars`: the mod jars it brings (path, sha1, md5, modId, version).
    incoming: Vec<serde_json::Value>,
    local: Vec<prism::history::Change>,
    /// Friends' files Obelisk blocked (signs of malware), and why.
    held: Vec<prism::held::Held>,
}

#[tauri::command]
async fn share_status(app: tauri::AppHandle, net: tauri::State<'_, ShareNet>, instance_id: String) -> Result<ShareStatus, String> {
    let game = game(&instance_id)?;
    let Some(share) = prism::share::get(&game) else {
        return Ok(ShareStatus { shared: false, me: String::new(), owner: String::new(), ended: prism::share::ended(&game), share: None, incoming: vec![], local: vec![], held: vec![] });
    };
    let me = share_net(&app, &net).await?.id();
    blocking(move || {
        // Don't ship whole baselines to the UI: the changelog is what it shows, plus the jars it brings.
        let incoming = prism::share::incoming(&game).into_iter().map(|mut c| {
            let jars = prism::share::jar_details(&game, &c);
            c.entries.clear();
            let mut v = serde_json::to_value(&c).unwrap_or_default();
            v["jars"] = jars.into();
            v
        }).collect();
        let owner = prism::share::owner(&game, &share);
        Ok(ShareStatus { shared: true, me, owner, ended: None, local: prism::share::local_changes(&game)?, held: prism::held::list(&game), incoming, share: Some(share) })
    }).await
}

/// How many incoming changes each shared pack has (for badges).
#[tauri::command]
async fn share_overview() -> Result<std::collections::HashMap<String, usize>, String> {
    blocking(|| Ok(prism::share::all(&prism::instances_dir(&prism::data_dir())).into_iter()
        .map(|(id, game, _)| (id, prism::share::incoming(&game).len())).collect())).await
}

#[tauri::command]
async fn share_start(app: tauri::AppHandle, net: tauri::State<'_, ShareNet>, instance_id: String) -> Result<String, String> {
    let game = game(&instance_id)?;
    let name = prism::list_instances(&prism::data_dir()).into_iter().find(|i| i.id == instance_id).map(|i| i.name).unwrap_or(instance_id);
    share_net(&app, &net).await?.share(&game, &name).await
}

/// Share → Get a code: list the pack's files (matching each to Modrinth/CurseForge by hash) and offer
/// it under a 6-character code for 30 minutes. Obelisk must stay open meanwhile.
#[tauri::command]
async fn pack_code_start(app: tauri::AppHandle, net: tauri::State<'_, ShareNet>, mr: tauri::State<'_, Mr>, instance_id: String) -> Result<serde_json::Value, String> {
    let game = game(&instance_id)?;
    let name = instance(&instance_id)?.name;
    let mr = mr.inner().clone();
    let (g, n) = (game.clone(), name.clone());
    let manifest = blocking(move || prism::packcode::manifest(&g, &n, Some(&mr), cf_client().as_ref())).await?;
    let (files, custom, private) = (manifest.files.len(), manifest.files.iter().filter(|f| f.custom).count(), manifest.private.len());
    let (code, expires) = share_net(&app, &net).await?.offer(&game, manifest).await?;
    Ok(serde_json::json!({ "code": code, "expiresMs": expires * 1000, "files": files, "custom": custom, "private": private }))
}

/// Share → code: friends connected, files sent, imports finished (None once the code is gone).
#[tauri::command]
async fn pack_code_status(app: tauri::AppHandle, net: tauri::State<'_, ShareNet>, code: String) -> Result<Option<prism::share_net::OfferStatus>, String> {
    Ok(share_net(&app, &net).await?.offer_status(&code))
}

#[tauri::command]
async fn pack_code_stop(app: tauri::AppHandle, net: tauri::State<'_, ShareNet>, code: String) -> Result<(), String> {
    share_net(&app, &net).await?.withdraw(&code);
    Ok(())
}

/// Import → code: what the pack holds, including its custom files (for the warning).
#[tauri::command]
async fn pack_code_preview(app: tauri::AppHandle, net: tauri::State<'_, ShareNet>, code: String) -> Result<prism::share_net::PackPreview, String> {
    use tauri::Emitter;
    prism::share_net::trace_start("checking a pack code", &app.package_info().version.to_string());
    let code = prism::packcode::normalize(&code)?;
    sweep_half_made().await;
    let cf = cf_client().map(std::sync::Arc::new);
    let a = app.clone();
    share_net(&app, &net).await?.preview(&code, cf, move |p| { let _ = a.emit("pack-import", p); }).await
}

/// Import → Stop: end the check or install under way (its half-made profile is removed).
#[tauri::command]
fn pack_code_cancel() {
    prism::share_net::stop_incoming();
}

/// Install a code's pack as a new profile; `with_custom` takes the files no site vouches for too.
/// Progress goes out as "pack-import" events ({done, total, file}). Returns the new profile and what was left out.
#[tauri::command]
async fn pack_code_import(app: tauri::AppHandle, net: tauri::State<'_, ShareNet>, code: String, with_custom: bool) -> Result<prism::share_net::ImportReport, String> {
    use tauri::Emitter;
    prism::share_net::trace_start("importing a pack code", &app.package_info().version.to_string());
    let code = prism::packcode::normalize(&code)?;
    sweep_half_made().await;
    let a = app.clone();
    let cf = cf_client().map(std::sync::Arc::new);
    let r = share_net(&app, &net).await?.import(&code, with_custom, cf, move |p| { let _ = a.emit("pack-import", p); }).await?;
    announce_held(&app, &r.id);
    Ok(r)
}

/// A join or import that failed half-way can leave a folder that isn't a profile (a file was in use while it was
/// removed): it goes to the Recycle Bin, so it can't block joining that pack again.
async fn sweep_half_made() {
    let _ = blocking(|| {
        let instances = prism::instances_dir(&prism::data_dir());
        for p in prism::share::half_made(&instances) {
            let _ = trash::delete(&p);
        }
        // Copies of a collaborative pack made before clones left collaboration behind: only the original keeps it.
        prism::share::dedupe(&instances);
        Ok(())
    }).await;
}

/// The window asks the user about files Obelisk blocked in this profile (held.rs), if any wait for an answer.
fn announce_held(app: &tauri::AppHandle, instance_id: &str) {
    use tauri::Emitter;
    let Ok(game) = game(instance_id) else { return };
    if prism::held::list(&game).iter().any(|h| !h.kept) {
        let _ = app.emit("held-files", serde_json::json!({ "id": instance_id }));
    }
}

/// Import/Join → Details: what the last import or join did, step by step (to see why one failed).
#[tauri::command]
fn share_trace() -> Vec<String> {
    prism::share_net::trace_log()
}

/// The files Obelisk blocked in a profile, and why.
#[tauri::command]
async fn held_list(instance_id: String) -> Result<Vec<prism::held::Held>, String> {
    Ok(prism::held::list(&game(&instance_id)?))
}

/// "Keep it blocked": the window stops asking about it (it stays listed on the profile's page).
#[tauri::command]
async fn held_keep(instance_id: String, path: String) -> Result<(), String> {
    prism::held::keep(&game(&instance_id)?, &path)
}

/// Share → Export as zip: the whole pack zipped into Downloads, then shown in Explorer.
/// Progress goes out as "pack-export" events ({done, total} in files). Returns the zip's path.
#[tauri::command]
async fn pack_export_zip(app: tauri::AppHandle, instance_id: String) -> Result<String, String> {
    use tauri::{Emitter, Manager};
    let game = game(&instance_id)?;
    let name = instance(&instance_id)?.name;
    let dir = app.path().download_dir().map_err(|e| format!("couldn't find your Downloads folder: {e}"))?;
    let a = app.clone();
    let out = blocking(move || {
        let out = prism::packcode::zip_name(&dir, &name);
        let mut last = 0;
        prism::packcode::export_zip(&game, &name, &out, |done, total| {
            if done - last >= 25 || done == total {
                last = done;
                let _ = a.emit("pack-export", serde_json::json!({ "done": done, "total": total }));
            }
        })?;
        Ok(out)
    }).await?;
    std::process::Command::new("explorer").arg(format!("/select,{}", out.display())).spawn().map_err(|e| e.to_string())?;
    Ok(out.display().to_string())
}

#[tauri::command]
async fn share_publish(app: tauri::AppHandle, net: tauri::State<'_, ShareNet>, instance_id: String, label: String) -> Result<bool, String> {
    let game = game(&instance_id)?;
    Ok(share_net(&app, &net).await?.publish(&game, &label)?.is_some())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Waiting { instance_id: String, changes: usize, auto_apply: bool }

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Synced { received: usize, waiting: Vec<Waiting> }

/// Pull from friends now. `received` counts changes that just arrived; `waiting` lists every pack with friends'
/// changes not applied yet (arrived now or earlier, e.g. by the background sync), and whether it applies them itself.
#[tauri::command]
async fn share_sync(app: tauri::AppHandle, net: tauri::State<'_, ShareNet>) -> Result<Synced, String> {
    let received = share_net(&app, &net).await?.sync_all().await.into_iter().map(|(_, n)| n).sum();
    let waiting = blocking(|| Ok(prism::share::all(&prism::instances_dir(&prism::data_dir())).into_iter()
        .map(|(id, game, s)| Waiting { instance_id: id, changes: prism::share::incoming(&game).len(), auto_apply: s.auto_apply })
        .filter(|w| w.changes > 0).collect())).await?;
    Ok(Synced { received, waiting })
}

#[tauri::command]
async fn share_apply(app: tauri::AppHandle, instance_id: String, ids: Vec<String>) -> Result<prism::share::Applied, String> {
    let game = game(&instance_id)?;
    let r = blocking(move || prism::share::apply(&game, &ids)).await?;
    if !r.blocked.is_empty() { announce_held(&app, &instance_id); }
    Ok(r)
}

/// Apply only one file (e.g. "mods/x.jar") of a friend's change; the rest of that change stays waiting.
#[tauri::command]
async fn share_apply_file(app: tauri::AppHandle, instance_id: String, id: String, path: String) -> Result<prism::share::Applied, String> {
    let game = game(&instance_id)?;
    let r = blocking(move || prism::share::apply_file(&game, &id, &path)).await?;
    if !r.blocked.is_empty() { announce_held(&app, &instance_id); }
    Ok(r)
}

#[tauri::command]
async fn share_skip(instance_id: String, id: String) -> Result<(), String> {
    prism::share::skip(&game(&instance_id)?, &id)
}

#[tauri::command]
async fn share_set_auto(instance_id: String, on: bool) -> Result<(), String> {
    let game = game(&instance_id)?;
    let mut s = prism::share::get(&game).ok_or("not shared")?;
    s.auto_apply = on;
    prism::share::save(&game, &s)
}

/// An error the window showed (state.svelte.ts toastError), for errors.jsonl.
#[tauri::command]
fn report_error(message: String, context: String) {
    errlog::record(if context.is_empty() { "window" } else { &context }, &message);
}

#[tauri::command]
async fn releases_call(command: String, args: serde_json::Value) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || releases::from_ui(&command, &args)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn share_set_auto_send(instance_id: String, on: bool) -> Result<(), String> {
    let game = game(&instance_id)?;
    let mut s = prism::share::get(&game).ok_or("not shared")?;
    s.auto_send = on;
    prism::share::save(&game, &s)
}

/// Collaborate → Blocked files → Allow: let one blocked file in. Only the user can: Windows itself asks them, in a
/// box that says why Obelisk blocked it, and the file is written only if they press Allow there. (Also never run
/// for an assistant: see bridge.rs.)
#[tauri::command]
async fn share_allow_held(app: tauri::AppHandle, window: tauri::WebviewWindow, instance_id: String, path: String) -> Result<bool, String> {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let game = game(&instance_id)?;
    let h = prism::held::list(&game).into_iter().find(|h| h.path == path).ok_or("That file isn't blocked any more.")?;
    let file = h.path.rsplit('/').next().unwrap_or(&h.path).to_string();
    let text = format!(
        "{} sent {file}, and Obelisk blocked it because it {}.

That is how programs that steal accounts and passwords behave.         Some real mods do it for their own sign-in features, so allow it only if you know this mod and trust where it came from.

        Allow {file} into this pack?",
        if h.from.is_empty() { "The person who shared this pack" } else { &h.from }, h.reasons.join(", and it "));
    let allow = blocking(move || Ok(app.dialog().message(text).title("Blocked file").kind(MessageDialogKind::Warning).parent(&window)
        .buttons(MessageDialogButtons::OkCancelCustom("Allow this file".into(), "Keep it blocked".into())).blocking_show())).await?;
    if allow {
        blocking(move || prism::held::allow(&game, &path)).await?;
    }
    Ok(allow)
}

/// The owner removes someone from a collaborative pack.
#[tauri::command]
async fn share_remove(app: tauri::AppHandle, net: tauri::State<'_, ShareNet>, instance_id: String, member: String) -> Result<(), String> {
    let game = game(&instance_id)?;
    share_net(&app, &net).await?.remove(&game, &member).map(|_| ())
}

/// The light view of a pack's collaboration for its page: who's in it, who owns it, everyone's wish lists,
/// or the note that the owner removed us. (share_status also works out local changes, which takes longer.)
#[tauri::command]
async fn share_info(app: tauri::AppHandle, net: tauri::State<'_, ShareNet>, instance_id: String) -> Result<serde_json::Value, String> {
    let game = game(&instance_id)?;
    let Some(s) = prism::share::get(&game) else {
        return Ok(serde_json::json!({ "shared": false, "ended": prism::share::ended(&game) }));
    };
    let me = share_net(&app, &net).await?.id();
    Ok(serde_json::json!({ "shared": true, "me": me, "owner": prism::share::owner(&game, &s), "members": s.members, "wishes": prism::share::wishes(&game) }))
}

/// Replace my own wish list for a collaborative pack.
#[tauri::command]
async fn share_set_wishes(app: tauri::AppHandle, net: tauri::State<'_, ShareNet>, instance_id: String, items: Vec<serde_json::Value>) -> Result<(), String> {
    let game = game(&instance_id)?;
    share_net(&app, &net).await?.set_wishes(&game, items)
}

/// The pack's whole change log, newest first: who changed what.
#[tauri::command]
async fn share_history(instance_id: String) -> Result<Vec<prism::share::Changeset>, String> {
    let game = game(&instance_id)?;
    blocking(move || Ok(prism::share::history_log(&game))).await
}

#[tauri::command]
async fn share_dismiss_ended(instance_id: String) -> Result<(), String> {
    prism::share::dismiss_ended(&game(&instance_id)?)
}

#[tauri::command]
async fn share_stop(instance_id: String) -> Result<(), String> {
    prism::share::stop(&game(&instance_id)?)
}

/// Collaborate → Get a code: a short code (like Share's) that carries the invite; the friend pastes it into Import.
#[tauri::command]
async fn collab_code(app: tauri::AppHandle, net: tauri::State<'_, ShareNet>, instance_id: String) -> Result<serde_json::Value, String> {
    sweep_half_made().await;
    let game = game(&instance_id)?;
    let name = instance(&instance_id)?.name;
    let (code, expires) = share_net(&app, &net).await?.offer_invite(&game, &name).await?;
    Ok(serde_json::json!({ "code": code, "expiresMs": expires * 1000 }))
}

#[tauri::command]
async fn share_join(app: tauri::AppHandle, net: tauri::State<'_, ShareNet>, code: String) -> Result<String, String> {
    prism::share_net::trace_start("joining a collaborative pack", &app.package_info().version.to_string());
    sweep_half_made().await;
    let id = share_net(&app, &net).await?.join(&code).await?;
    announce_held(&app, &id);
    Ok(id)
}

// ---- backups (change-based; see core/src/history.rs)

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupState {
    backups: Vec<prism::history::Backup>,
    /// Changes since the newest backup.
    pending: Vec<prism::history::Change>,
    disk_bytes: u64,
}

#[tauri::command]
async fn backups(instance_id: String) -> Result<BackupState, String> {
    let game = game(&instance_id)?;
    blocking(move || Ok(BackupState {
        pending: prism::history::pending(&game)?,
        backups: prism::history::list(&game),
        disk_bytes: prism::history::disk_use(&game),
    })).await
}

/// A checkpoint (manual backup), taken even when nothing changed.
#[tauri::command]
async fn create_checkpoint(app: tauri::AppHandle, instance_id: String, label: String) -> Result<prism::history::Backup, String> {
    let game = game(&instance_id)?;
    let (keep_auto, keep_manual) = backup_limits(&app);
    blocking(move || prism::work::run("Taking a backup", || {
        let label = if label.trim().is_empty() { "Checkpoint".to_string() } else { label.trim().to_string() };
        let b = prism::history::create(&game, "manual", &label, false)?.ok_or("nothing to back up")?;
        prism::history::prune(&game, keep_auto, keep_manual)?;
        Ok(b)
    })).await
}

/// The noon/midnight backup: `slot_ms` is when the current slot began (the UI knows the time zone).
/// Taken only if the pack changed and the slot doesn't have one yet.
#[tauri::command]
async fn auto_backup(app: tauri::AppHandle, instance_id: String, slot_ms: u64) -> Result<bool, String> {
    let game = game(&instance_id)?;
    let (keep_auto, keep_manual) = backup_limits(&app);
    blocking(move || {
        let took = prism::history::auto(&game, slot_ms)?.is_some();
        if took {
            prism::history::prune(&game, keep_auto, keep_manual)?;
        }
        Ok(took)
    }).await
}

/// A backup only if the pack changed since the newest one (Obelisk start; also before every launch).
#[tauri::command]
async fn change_backup(app: tauri::AppHandle, instance_id: String, label: String) -> Result<bool, String> {
    let game = game(&instance_id)?;
    let (keep_auto, keep_manual) = backup_limits(&app);
    blocking(move || {
        let took = prism::history::on_change(&game, &label)?.is_some();
        if took { prism::history::prune(&game, keep_auto, keep_manual)?; }
        Ok(took)
    }).await
}

/// A backup taken right before something risky (Update all, purging configs).
#[tauri::command]
async fn safety_backup(app: tauri::AppHandle, instance_id: String, label: String) -> Result<(), String> {
    let game = game(&instance_id)?;
    let (keep_auto, keep_manual) = backup_limits(&app);
    blocking(move || prism::work::run("Taking a safety backup", || {
        prism::history::create(&game, "safety", &label, true)?;
        prism::history::prune(&game, keep_auto, keep_manual).map(|_| ())
    })).await
}

#[tauri::command]
async fn restore_backup(instance_id: String, backup_id: String) -> Result<Vec<prism::history::Change>, String> {
    let game = game(&instance_id)?;
    blocking(move || prism::work::run("Restoring the backup", || prism::history::restore(&game, &backup_id))).await
}

#[tauri::command]
async fn revert_change(instance_id: String, backup_id: String, path: String) -> Result<(), String> {
    let game = game(&instance_id)?;
    blocking(move || prism::history::revert_change(&game, &backup_id, &path)).await
}

#[tauri::command]
async fn prism_defaults() -> prism::PrismDefaults {
    prism::prism_defaults(&prism::data_dir())
}

#[tauri::command]
async fn set_prism_defaults(defaults: prism::PrismDefaults) -> Result<(), String> {
    prism::set_prism_defaults(&prism::data_dir(), &defaults)
}

/// Prism rewrites its config files when it exits, so edits should happen while it's closed.
#[tauri::command]
async fn prism_running() -> bool {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    sys.processes().values().any(|p| p.name().eq_ignore_ascii_case("prismlauncher.exe"))
}

#[tauri::command]
async fn list_accounts() -> Vec<prism::AccountEntry> {
    prism::list_accounts(&prism::data_dir())
}

async fn while_prism_closed(f: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    if prism_running().await {
        return Err("Close Prism first. It keeps its own copy of the account list and would undo this.".into());
    }
    f()
}

#[tauri::command]
async fn switch_account(uuid: String) -> Result<(), String> {
    while_prism_closed(|| prism::set_active_account(&prism::data_dir(), &uuid)).await
}

#[tauri::command]
async fn logout_account(uuid: String) -> Result<(), String> {
    while_prism_closed(|| prism::remove_account(&prism::data_dir(), &uuid)).await
}

#[tauri::command]
async fn system_memory_mb() -> u64 {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.total_memory() / (1024 * 1024)
}

fn settings_path(_app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(datadir::root().join("settings.json"))
}

#[tauri::command]
async fn get_settings(app: tauri::AppHandle) -> serde_json::Value {
    settings_value(&app)
}

fn settings_value(app: &tauri::AppHandle) -> serde_json::Value {
    settings_path(app).ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

#[tauri::command]
async fn set_settings(app: tauri::AppHandle, settings: serde_json::Value) -> Result<(), String> {
    let p = settings_path(&app)?;
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(p, serde_json::to_string_pretty(&settings).unwrap()).map_err(|e| e.to_string())
}

/// Saved-for-later projects (the heart on a project page), kept apart from settings.
#[tauri::command]
async fn get_favorites() -> serde_json::Value {
    std::fs::read_to_string(datadir::root().join("favorites.json")).ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

#[tauri::command]
async fn set_favorites(favorites: serde_json::Value) -> Result<(), String> {
    let p = datadir::root().join("favorites.json");
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&favorites).unwrap()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

// ---- CurseForge API key: kept in Windows Credential Manager, never sent back to the UI ----

fn cf_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new("Obelisk", "curseforge-api-key").map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_cf_key(key: String) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() { cf_entry()?.delete_credential().or(Ok::<(), keyring::Error>(())).map_err(|e| e.to_string()) }
    else { cf_entry()?.set_password(key).map_err(|e| e.to_string()) }
}

#[tauri::command]
async fn has_cf_key() -> bool {
    cf_entry().ok().and_then(|e| e.get_password().ok()).is_some_and(|k| !k.is_empty())
}

fn cf_client() -> Option<prism::curseforge::Client> {
    let key = cf_entry().ok()?.get_password().ok().filter(|k| !k.is_empty())?;
    prism::curseforge::Client::new(&key).ok()
}

fn cf_required() -> Result<prism::curseforge::Client, String> {
    cf_client().ok_or_else(|| "Add your CurseForge API key in Settings → CurseForge API first.".into())
}

fn instance(id: &str) -> Result<Instance, String> {
    prism::list_instances(&prism::data_dir()).into_iter().find(|i| i.id == id).ok_or_else(|| "instance not found".into())
}

/// Source, popularity, client/server side and available update for every file in a profile.
#[tauri::command]
async fn check_updates(app: tauri::AppHandle, mr: tauri::State<'_, Mr>, instance_id: String) -> Result<Vec<prism::updates::FileInfo>, String> {
    let mr = mr.inner().clone();
    let inst = instance(&instance_id)?;
    let (game, cache) = (game(&instance_id)?, hash_cache(&app)?);
    blocking(move || prism::updates::check(&game, &cache, &mr, cf_client().as_ref(), &inst.loader, &inst.minecraft)).await
}

/// What an install or update needs from the user when CurseForge blocks direct downloads.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ManualDownload {
    name: String,
    page: String,
    file_name: String,
    sha1: String,
    size: u64,
    folder: String,
    index_toml: String,
    replaces: Option<String>,
}

#[tauri::command]
async fn apply_update(mr: tauri::State<'_, Mr>, instance_id: String, info: serde_json::Value) -> Result<Option<ManualDownload>, String> {
    let mr = mr.inner().clone();
    let inst = instance(&instance_id)?;
    let game = game(&instance_id)?;
    let s = |k: &str| info[k].as_str().unwrap_or_default().to_string();
    let (kind, file_name, provider, project_id) = (s("kind"), s("fileName"), s("provider"), s("projectId"));
    let update_id = info["update"]["id"].as_str().ok_or("no update")?.to_string();
    blocking(move || {
        if provider == "modrinth" {
            let project_type = match kind.as_str() { "resourcepacks" => "resourcepack", "shaderpacks" => "shader", _ => "mod" };
            let req = prism::modrinth::InstallRequest { project_id, project_type: project_type.into(), version_id: Some(update_id), loader_override: None, force: false };
            let existing = vec![prism::modrinth::Installed { project_id: req.project_id.clone(), version_id: String::new(), kind: kind.clone(), file_name }];
            mr.install(&game, &req, &inst.loader, &inst.minecraft, &existing)?;
            return Ok(None);
        }
        let cf = cf_required()?;
        let (mod_id, file_id) = (project_id.parse().map_err(|_| "bad project id")?, update_id.parse().map_err(|_| "bad file id")?);
        match cf.replace(&game, &kind, &file_name, mod_id, file_id)? {
            None => Ok(None),
            Some(file) => {
                let project = cf.project(mod_id)?;
                Ok(Some(manual(&project, &file, &kind, Some(file_name))))
            }
        }
    }).await
}

fn manual(project: &serde_json::Value, file: &serde_json::Value, folder: &str, replaces: Option<String>) -> ManualDownload {
    ManualDownload {
        name: project["name"].as_str().unwrap_or_default().into(),
        page: format!("{}/files/{}", project["links"]["websiteUrl"].as_str().unwrap_or_default().trim_end_matches('/'), file["id"]),
        file_name: file["fileName"].as_str().unwrap_or_default().into(),
        sha1: prism::curseforge::sha1_of(file),
        size: file["fileLength"].as_u64().unwrap_or(0),
        folder: folder.into(),
        index_toml: prism::curseforge::index_toml(project, file),
        replaces,
    }
}

/// Wait (in the background) for the user's browser download, then move it into the profile.
/// Emits `manual-download` {name, ok, error}.
#[tauri::command]
async fn await_download(app: tauri::AppHandle, instance_id: String, job: serde_json::Value) -> Result<(), String> {
    use tauri::Emitter;
    let mut game = game(&instance_id)?;
    // Data packs: the job names the world they go into.
    let world = job["world"].as_str().filter(|w| !w.is_empty());
    if let Some(w) = world {
        game = prism::world_dir(&game, w)?;
    }
    let downloads = app.path().download_dir().map_err(|e| e.to_string())?;
    let s = |k: &str| job[k].as_str().unwrap_or_default().to_string();
    let (name, file_name, sha1, folder, index_toml) = (s("name"), s("fileName"), s("sha1"), s("folder"), s("indexToml"));
    let replaces = job["replaces"].as_str().map(str::to_string);
    let size = job["size"].as_u64().unwrap_or(0);
    let folder_ok = if world.is_some() { folder == "datapacks" } else { prism::KINDS.contains(&folder.as_str()) };
    if !folder_ok || file_name.contains(['/', '\\']) || file_name.contains("..") {
        return Err("invalid download".into());
    }
    std::thread::spawn(move || {
        let dir = game.join(&folder);
        let result = prism::curseforge::wait_for_download(&downloads, size, &sha1, &dir.join(&file_name), std::time::Duration::from_secs(900))
            .and_then(|_| {
                if let Some(old) = replaces.filter(|o| *o != file_name) {
                    prism::delete_content(&game, &folder, &old, |p| std::fs::remove_file(p).map_err(|e| e.to_string()))?;
                }
                let stem = file_name.trim_end_matches(".jar").trim_end_matches(".zip").replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "-");
                std::fs::create_dir_all(dir.join(".index")).map_err(|e| e.to_string())?;
                std::fs::write(dir.join(".index").join(format!("{stem}.pw.toml")), index_toml).map_err(|e| e.to_string())
            });
        let _ = app.emit("manual-download", serde_json::json!({ "name": name, "ok": result.is_ok(), "error": result.err() }));
    });
    Ok(())
}

// ---- Config editor ----

#[tauri::command]
async fn mod_configs(instance_id: String, file_name: String) -> Result<Vec<prism::configs::ConfigFile>, String> {
    let game = game(&instance_id)?;
    blocking(move || prism::configs::find(&game, &file_name)).await
}

#[tauri::command]
async fn read_config(instance_id: String, path: String) -> Result<String, String> {
    prism::configs::read(&game(&instance_id)?, &path)
}

// ---- Diagnosis ----

#[tauri::command]
async fn crash_reports(instance_id: String) -> Result<Vec<prism::crash::ReportInfo>, String> {
    Ok(prism::crash::list(&game(&instance_id)?))
}

#[tauri::command]
async fn read_report(instance_id: String, file: String) -> Result<String, String> {
    let game = game(&instance_id)?;
    blocking(move || prism::crash::read(&game, &file)).await
}

#[tauri::command]
async fn diagnose_crash(instance_id: String, file: String) -> Result<prism::crash::Diagnosis, String> {
    let game = game(&instance_id)?;
    blocking(move || prism::crash::diagnose(&game, &file)).await
}

/// Profiles whose duplicates the user chose to ignore: instance id -> those duplicate files then.
/// Kept apart from settings.json, which the UI rewrites whole.
fn ignored_dupes_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(settings_path(app)?.with_file_name("ignored-duplicates.json"))
}

fn dupes_signature(found: &[prism::dupes::Duplicate]) -> String {
    found.iter().flat_map(|d| d.files.iter().cloned()).collect::<Vec<_>>().join("|")
}

/// Ignoring lasts only while the duplicates are the same files: a new one warns again.
fn dupes_ignored(app: &tauri::AppHandle, instance_id: &str, found: &[prism::dupes::Duplicate]) -> bool {
    let saved: serde_json::Value = ignored_dupes_path(app).ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    !found.is_empty() && saved.get(instance_id).and_then(|v| v.as_str()) == Some(&dupes_signature(found))
}

#[derive(serde::Serialize)]
struct DupeState { dupes: Vec<prism::dupes::Duplicate>, ignored: bool }

/// Mods present twice among the profile's enabled jars (same mod id, e.g. one from each site).
#[tauri::command]
async fn duplicate_mods(app: tauri::AppHandle, instance_id: String) -> Result<DupeState, String> {
    let game = game(&instance_id)?;
    let dupes = blocking(move || Ok(prism::dupes::find(&game))).await?;
    let ignored = dupes_ignored(&app, &instance_id, &dupes);
    Ok(DupeState { dupes, ignored })
}

/// Let the profile start despite its current duplicates (`on`), or warn about them again.
#[tauri::command]
async fn ignore_duplicates(app: tauri::AppHandle, instance_id: String, on: bool) -> Result<(), String> {
    let game = game(&instance_id)?;
    let found = blocking(move || Ok(prism::dupes::find(&game))).await?;
    let p = ignored_dupes_path(&app)?;
    let mut saved: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(&p).ok()
        .and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if on && !found.is_empty() { saved.insert(instance_id, dupes_signature(&found).into()); } else { saved.remove(&instance_id); }
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(p, serde_json::to_string_pretty(&saved).unwrap()).map_err(|e| e.to_string())
}

/// Lightning plugin: whether the mod is in this profile and its latest capture results.
#[tauri::command]
async fn lightning_runs(instance_id: String) -> Result<serde_json::Value, String> {
    let game = game(&instance_id)?;
    blocking(move || Ok(serde_json::json!({
        "installed": prism::lightning::installed(&game),
        "runs": prism::lightning::runs(&game, 25),
    }))).await
}

/// Recent sessions' load times, newest first, each split into phases.
#[tauri::command]
async fn load_times(instance_id: String) -> Result<serde_json::Value, String> {
    let game = game(&instance_id)?;
    blocking(move || Ok(serde_json::Value::Array(prism::timeline::sessions(&game, 30).iter().map(|s| serde_json::json!({
        "clickMs": s.click_ms, "source": s.source, "label": s.label, "speedloader": s.speedloader.is_some(),
        "phases": prism::timeline::phases(s),
    })).collect()))).await
}

/// Open a crash report, log or Lightning report with the system's default app.
#[tauri::command]
async fn open_game_file(app: tauri::AppHandle, instance_id: String, rel: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let ok = !rel.contains("..") && (rel.starts_with("crash-reports/") || rel.starts_with("hs_err_pid") || rel == "logs/latest.log"
        || (rel.starts_with("lightning/captures/") && rel.ends_with(".html")));
    if !ok {
        return Err("can't open that file".into());
    }
    let path = game(&instance_id)?.join(&rel);
    app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_pins(instance_id: String) -> Result<Vec<String>, String> {
    Ok(prism::pins(&game(&instance_id)?))
}

#[tauri::command]
async fn set_pins(instance_id: String, pins: Vec<String>) -> Result<(), String> {
    prism::set_pins(&game(&instance_id)?, &pins)
}

#[tauri::command]
async fn config_default(instance_id: String, path: String) -> Result<Option<String>, String> {
    prism::configs::default_text(&game(&instance_id)?, &path)
}

#[tauri::command]
async fn restore_config_default(instance_id: String, path: String) -> Result<String, String> {
    prism::configs::restore_default(&game(&instance_id)?, &path)
}

/// Config entries no installed mod owns (scans every jar, so it takes a few seconds).
#[tauri::command]
async fn orphan_configs(instance_id: String) -> Result<Vec<String>, String> {
    let game = game(&instance_id)?;
    blocking(move || Ok(prism::configs::orphans(&game))).await
}

/// Sort a profile's mods into client / server / both. `platform` is what the download sites say
/// (file name -> "client" | "server" | "both"), from the update check the UI already has.
#[tauri::command]
async fn mod_sides(instance_id: String, platform: std::collections::HashMap<String, String>) -> Result<Vec<prism::sides::ModSide>, String> {
    let game = game(&instance_id)?;
    blocking(move || Ok(prism::sides::classify(&game, &platform))).await
}

/// Write the server pack (folder + zip, loader installer and start scripts) and the client pack.
#[tauri::command]
async fn build_packs(mr: tauri::State<'_, Mr>, instance_id: String, plan: prism::sides::PackPlan) -> Result<prism::sides::Built, String> {
    let game = game(&instance_id)?;
    let dir = prism::instances_dir(&prism::data_dir()).join(&instance_id);
    let mr = mr.inner().clone();
    blocking(move || prism::sides::build(&dir, &game, &plan, &|url| mr.fetch(url))).await
}

/// Delete the chosen orphaned configs (to the Recycle Bin).
#[tauri::command]
async fn clean_configs(instance_id: String, names: Vec<String>) -> Result<usize, String> {
    let game = game(&instance_id)?;
    blocking(move || {
        // Backups cover the config folder, so a purge can be undone from the Backups tab.
        prism::history::create(&game, "safety", "Before purging configs", true)?;
        prism::configs::remove_orphans(&game, &names, |p| trash::delete(p).map_err(|e| e.to_string()))
    }).await
}

/// Open Explorer with this file selected.
#[tauri::command]
async fn reveal_file(instance_id: String, kind: String, file_name: String) -> Result<(), String> {
    if !["mods", "resourcepacks", "shaderpacks"].contains(&kind.as_str()) || file_name.contains(['/', '\\']) || file_name.contains("..") {
        return Err("invalid path".into());
    }
    let folder = game(&instance_id)?.join(&kind);
    let path = folder.join(&file_name);
    // Gone (replaced outside Obelisk, e.g. a newer version dropped in): open the folder instead of nothing.
    let arg = if path.exists() { format!("/select,{}", path.to_string_lossy().replace('/', "\\")) } else { folder.to_string_lossy().replace('/', "\\") };
    std::process::Command::new("explorer").arg(arg).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Show a built server/client pack in Explorer (only the packs Obelisk writes).
#[tauri::command]
async fn reveal_pack(path: String) -> Result<(), String> {
    let p = std::path::PathBuf::from(&path);
    let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
    if !(name.ends_with(" Server") || name.ends_with(" Server.zip") || name.ends_with(" Client.zip")) || !p.exists() {
        return Err("not a pack Obelisk built".into());
    }
    let path = path.replace('/', "\\");
    std::process::Command::new("explorer").arg(format!("/select,{path}")).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn write_config(instance_id: String, path: String, text: String) -> Result<(), String> {
    prism::configs::write(&game(&instance_id)?, &path, &text)
}

#[tauri::command]
async fn cf_search(query: prism::curseforge::SearchQuery) -> Result<serde_json::Value, String> {
    let cf = cf_required()?;
    blocking(move || cf.search(&query)).await
}

#[tauri::command]
async fn cf_author(id: u64) -> Result<Vec<serde_json::Value>, String> {
    let cf = cf_required()?;
    blocking(move || cf.author_projects(id)).await
}

#[tauri::command]
async fn cf_project(id: u64) -> Result<serde_json::Value, String> {
    let cf = cf_required()?;
    blocking(move || Ok(serde_json::json!({ "project": cf.project(id)?, "description": cf.description(id)? }))).await
}

/// The same project on the other site: {mr, cf} ids (either may be null), proven by a shared file where possible.
#[tauri::command]
async fn cross_site(mr: tauri::State<'_, Mr>, id: String) -> Result<serde_json::Value, String> {
    let Some(cf) = cf_client() else {
        return Ok(serde_json::json!({ "mr": (!id.starts_with("cf:")).then_some(id), "cf": null }));
    };
    let mr = mr.inner().clone();
    blocking(move || Ok(match id.strip_prefix("cf:").and_then(|n| n.parse::<u64>().ok()) {
        Some(c) => serde_json::json!({ "mr": prism::crosssite::cf_to_mr(&cf, &mr, c).unwrap_or(None), "cf": c }),
        None => serde_json::json!({ "mr": id, "cf": prism::crosssite::mr_to_cf(&cf, &mr, &id).unwrap_or(None) }),
    })).await
}

#[tauri::command]
async fn cf_id_by_slug(slug: String) -> Result<Option<u64>, String> {
    let cf = cf_required()?;
    blocking(move || cf.id_by_slug(&slug)).await
}

#[tauri::command]
async fn cf_files(id: u64) -> Result<Vec<serde_json::Value>, String> {
    let cf = cf_required()?;
    blocking(move || cf.files(id, "", "")).await
}

#[tauri::command]
async fn cf_changelog(mod_id: u64, file_id: u64) -> Result<String, String> {
    let cf = cf_required()?;
    blocking(move || cf.changelog(mod_id, file_id)).await
}

/// Install a CurseForge project into a profile. Returns a manual download when the author blocks direct downloads.
#[tauri::command]
async fn cf_install(app: tauri::AppHandle, mr: tauri::State<'_, Mr>, instance_id: String, mod_id: u64, file_id: Option<u64>, world: Option<String>, loader_override: Option<String>) -> Result<serde_json::Value, String> {
    let cf = cf_required()?;
    let mr = mr.inner().clone();
    let inst = instance(&instance_id)?;
    let (game, cache) = (game(&instance_id)?, hash_cache(&app)?);
    // A Fabric mod on NeoForge through Connector: Fabric files and dependencies, with Forgified Fabric API standing
    // in for Fabric API.
    let loader = loader_override.unwrap_or_else(|| inst.loader.clone());
    let cross = !loader.eq_ignore_ascii_case(&inst.loader);
    let skip = move |mut s: std::collections::HashSet<u64>| { if cross { s.insert(prism::curseforge::FABRIC_API); } s };
    blocking(move || {
        if let Some(world) = world {
            let root = prism::world_dir(&game, &world)?;
            let outcome = cf.install(&root, mod_id, file_id, &loader, &inst.minecraft, &skip(Default::default()))?;
            let mut v = serde_json::to_value(outcome).map_err(|e| e.to_string())?;
            v["world"] = world.into();
            return Ok(v);
        }
        // Everything already in the profile, whichever site it came from, counts as installed.
        let known: std::collections::HashSet<u64> = prism::updates::check(&game, &cache, &mr, Some(&cf), &inst.loader, &inst.minecraft)?
            .into_iter().filter(|i| i.provider == "curseforge").filter_map(|i| i.project_id.parse().ok()).collect();
        let outcome = cf.install(&game, mod_id, file_id, &loader, &inst.minecraft, &skip(known))?;
        serde_json::to_value(outcome).map_err(|e| e.to_string())
    }).await
}

#[tauri::command]
async fn cf_featured() -> Result<serde_json::Value, String> {
    let cf = cf_required()?;
    blocking(move || cf.featured(None)).await
}

/// Windows 11 draws a 1px accent-coloured border round undecorated windows; turn it off, and ask for
/// rounded corners (the frame trick Tauri uses for a shadow also leaves a 1px accent line on top, so
/// the window is created without it).
fn no_accent_border(hwnd: isize) {
    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(hwnd: isize, attr: u32, value: *const std::ffi::c_void, size: u32) -> i32;
    }
    const DWMWA_BORDER_COLOR: u32 = 34;
    const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;
    // Fails harmlessly on Windows 10, which has no such border.
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: u32 = 2;
    unsafe {
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &DWMWA_COLOR_NONE as *const u32 as *const _, 4);
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_ROUND as *const u32 as *const _, 4);
    }
}

/// Tauri sets only the small window icon, so the taskbar falls back to Windows' cached exe icon
/// (which can be stale after an update). Give the window the big icon from our own exe.
fn set_big_icon(hwnd: isize) {
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ExtractIconExW(file: *const u16, index: i32, large: *mut isize, small: *mut isize, n: u32) -> u32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SendMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let wide: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    let (mut large, mut small) = (0isize, 0isize);
    unsafe {
        if ExtractIconExW(wide.as_ptr(), 0, &mut large, &mut small, 1) > 0 && large != 0 {
            SendMessageW(hwnd, 0x0080 /* WM_SETICON */, 1 /* ICON_BIG */, large);
        }
    }
}

trait NoWindow {
    fn creation_flags_no_window(&mut self) -> &mut Self;
}
impl NoWindow for std::process::Command {
    fn creation_flags_no_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
    }
}

/// Bring up Prism's own window (accounts, instance editing).
#[tauri::command]
async fn open_prism() -> Result<(), String> {
    std::process::Command::new(prism::launcher_exe()).spawn().map(|_| ()).map_err(|e| format!("could not start Prism: {e}"))
}

/// Hand a modpack (URL or local .zip/.mrpack) to Prism's importer.
#[tauri::command]
async fn import_pack(source: String) -> Result<(), String> {
    // A pack file from anywhere is opened and scanned first, like a share code's custom files.
    let path = std::path::PathBuf::from(&source);
    if path.is_file() {
        let p = path.clone();
        blocking(move || prism::work::run("Checking the pack file", || {
            let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
            let found = prism::guard::inspect(&p.to_string_lossy(), &bytes);
            if !found.serious.is_empty() {
                return Err(format!("Not importing this pack: it {}.", found.serious.join("; ")));
            }
            if let prism::guard::Scan::Threat(t) = prism::guard::defender_scan(&p) {
                return Err(format!("Not importing this pack: Windows Defender found a threat ({t})."));
            }
            Ok(())
        })).await?;
    }
    std::process::Command::new(prism::launcher_exe())
        .args(["--import", &source])
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not start Prism: {e}"))
}

/// Launch through Prism, which handles the account, Java and the loader. Quiet launch (the default)
/// hides Prism's "Please wait..." progress box with its download bars and Abort button; anything
/// else Prism shows (sign-in, errors, the console on a crash) stays visible. Emits "game-started"
/// {id, up} once the game window opens (up = true), or when Prism gives up (up = false).
#[tauri::command]
/// Returns a note when the profile's pinned Java was wrong and got switched first.
async fn launch(app: tauri::AppHandle, instance_id: String) -> Result<Option<String>, String> {
    // A player runs one game at a time. Test copies the assistant runs under C:\mcw aren't modpacks, so they don't count.
    static LAUNCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    struct Starting(bool);
    impl Drop for Starting {
        fn drop(&mut self) { if self.0 { LAUNCHING.store(false, std::sync::atomic::Ordering::SeqCst); } }
    }
    if LAUNCHING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Err("Another game is starting. Wait for it to open, or close it first.".into());
    }
    let mut starting = Starting(true);
    CANCEL.store(false, std::sync::atomic::Ordering::SeqCst);
    if let Some(other) = blocking(|| Ok(running_java())).await?.into_iter().map(|(id, _)| id).find(|id| game(id).is_ok()) {
        let name = prism::list_instances(&prism::data_dir()).into_iter().find(|i| i.id == other).map(|i| i.name).unwrap_or(other);
        return Err(format!("{name} is already running. Close it before starting another game."));
    }
    let game_dir = game(&instance_id)?;
    let g = game_dir.clone();
    let found = blocking(move || Ok(prism::dupes::find(&g))).await?;
    if !dupes_ignored(&app, &instance_id, &found) { prism::dupes::check_before_launch(&game_dir)?; }
    let java_note = prism::java::check_before_launch(&prism::data_dir(), &instance_id).unwrap_or(None);
    let quiet = settings_value(&app).get("quietLaunch").and_then(|v| v.as_bool()).unwrap_or(true);
    let shaders_off = settings_value(&app).get("shadersOffAtStart").and_then(|v| v.as_bool()).unwrap_or(true);
    // Pack changed since the last backup (by hand, another tool, a chat)? Back it up before the game touches anything.
    if settings_value(&app).pointer("/backups/auto").and_then(|v| v.as_bool()).unwrap_or(true) {
        let (keep_auto, keep_manual) = backup_limits(&app);
        let g = game_dir.clone();
        let _ = blocking(move || {
            if prism::history::on_change(&g, "Before launch")?.is_some() { prism::history::prune(&g, keep_auto, keep_manual)?; }
            Ok(())
        }).await;
    }
    // After the backup above, so the shader setting as it was is kept in Backups.
    if shaders_off {
        prism::shaders_off(&game_dir)?;
    }
    // Paths an older Obelisk wrote with bare backslashes make Prism find no packs (it then shows its own window).
    let data = prism::data_dir();
    let healed = prism::heal_cfg(&data.join("prismlauncher.cfg")).unwrap_or(false);
    let inst_cfg = game_dir.parent().unwrap_or(&game_dir).join("instance.cfg");
    let _ = prism::heal_cfg(&inst_cfg);
    if quiet {
        // Prism pops its console window when a game crashes; Obelisk reports crashes itself (Diagnosis).
        let off = |k: &'static str, v: &str| (k, Some(v.to_string()));
        prism::update_cfg(&inst_cfg, &[off("OverrideConsole", "true"), off("ShowConsole", "false"), off("ShowConsoleOnError", "false"), off("AutoCloseConsole", "true")])?;
    }
    if healed { close_idle_prism().await?; }
    let before = game_windows();
    let click_ms = prism::timeline::now_ms();
    // Cancelled during the checks and backup above: don't start it at all.
    if CANCEL.swap(false, std::sync::atomic::Ordering::SeqCst) {
        return Err("Cancelled.".into());
    }
    let mut child = std::process::Command::new(prism::launcher_exe())
        .args(["--launch", &instance_id])
        .spawn()
        .map_err(|e| format!("could not start Prism: {e}"))?;
    starting.0 = false; // handed to the watcher, which clears it once the game is up or has failed to start
    std::thread::spawn(move || {
        use tauri::Emitter;
        let found = watch_launch(&app, &instance_id, quiet, &before);
        LAUNCHING.store(false, std::sync::atomic::Ordering::SeqCst);
        if CANCEL.swap(false, std::sync::atomic::Ordering::SeqCst) {
            let _ = app.emit("game-started", serde_json::json!({ "id": instance_id, "up": false, "cancelled": true }));
            return;
        }
        // Prism closed without a game window: say why, from its exit code and its own log (never a silent stop).
        let reason = found.is_none().then(|| {
            let code = child.try_wait().ok().flatten().and_then(|s| s.code());
            let mut why = match code {
                Some(c) if c != 0 => format!("Prism closed with error code {c} before the game started."),
                Some(_) => "Prism closed before the game started.".to_string(),
                None => "The game window didn't appear.".to_string(),
            };
            if let Some(r) = prism::prism_stop_reason(&prism::data_dir(), &instance_id) {
                why += &format!(" {r}");
            }
            why
        });
        let _ = app.emit("game-started", serde_json::json!({ "id": instance_id, "up": found.is_some(), "reason": reason }));
        // A crash report written since Play was clicked: the UI offers its diagnosis.
        let crashed = |app: &tauri::AppHandle| {
            if let Some(r) = prism::crash::list(&game_dir).into_iter().filter(|r| r.kind != "log" && r.time_ms as i64 >= click_ms).max_by_key(|r| r.time_ms) {
                let _ = app.emit("game-crashed", serde_json::json!({ "id": instance_id, "file": r.file }));
            }
        };
        if found.is_none() { crashed(&app); }
        // Then tell the UI when the game closes (normally, crashed or killed).
        if let Some((pid, hwnd)) = found {
            let window_ms = prism::timeline::now_ms();
            let java_ms = prism::timeline::process_start_ms(pid);
            if prism::window::obelisk_borderless(&game_dir) {
                make_borderless(hwnd);
            }
            let pid = sysinfo::Pid::from_u32(pid);
            let mut sys = sysinfo::System::new();
            while sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true) > 0 && sys.process(pid).is_some() {
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            // Load times for the Diagnosis tab and Speed Loader (obelisk-timeline.jsonl in the game folder).
            prism::timeline::record(&game_dir, prism::timeline::Session { source: "obelisk".into(), click_ms, java_ms, window_ms: Some(window_ms),
                exit_ms: Some(prism::timeline::now_ms()), ..Default::default() });
            let _ = app.emit("game-exited", serde_json::json!({ "id": instance_id }));
            crashed(&app);
        }
    });
    Ok(java_note)
}

/// Borderless fullscreen for packs without a mod for it: drop the frame and cover the monitor the
/// window is on. The game sees an ordinary resize.
fn make_borderless(hwnd: isize) {
    #[repr(C)]
    struct Rect { l: i32, t: i32, r: i32, b: i32 }
    #[repr(C)]
    struct MonitorInfo { size: u32, monitor: Rect, work: Rect, flags: u32 }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetWindowLongPtrW(h: isize, i: i32) -> isize;
        fn SetWindowLongPtrW(h: isize, i: i32, v: isize) -> isize;
        fn MonitorFromWindow(h: isize, f: u32) -> isize;
        fn GetMonitorInfoW(m: isize, i: *mut MonitorInfo) -> i32;
        fn SetWindowPos(h: isize, after: isize, x: i32, y: i32, cx: i32, cy: i32, f: u32) -> i32;
        fn ShowWindow(h: isize, cmd: i32) -> i32;
    }
    const GWL_STYLE: i32 = -16;
    const FRAME: isize = 0x00C0_0000 | 0x0004_0000 | 0x0002_0000 | 0x0001_0000 | 0x0008_0000; // caption, sizing frame, min, max, system menu
    unsafe {
        ShowWindow(hwnd, 9 /* SW_RESTORE */);
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(hwnd, GWL_STYLE, style & !FRAME);
        let mut mi = MonitorInfo { size: std::mem::size_of::<MonitorInfo>() as u32, monitor: Rect { l: 0, t: 0, r: 0, b: 0 }, work: Rect { l: 0, t: 0, r: 0, b: 0 }, flags: 0 };
        if GetMonitorInfoW(MonitorFromWindow(hwnd, 2 /* NEAREST */), &mut mi) != 0 {
            let m = mi.monitor;
            SetWindowPos(hwnd, 0, m.l, m.t, m.r - m.l, m.b - m.t, 0x0020 | 0x0040 | 0x0004 /* FRAMECHANGED | SHOWWINDOW | NOZORDER */);
        }
    }
}

#[tauri::command]
async fn window_mode(instance_id: String) -> Result<prism::window::WindowMode, String> {
    Ok(prism::window::get(&game(&instance_id)?))
}

#[tauri::command]
async fn set_window_mode(instance_id: String, mode: prism::window::Mode) -> Result<(), String> {
    prism::window::set(&game(&instance_id)?, mode)
}

/// The same window mode for every profile; returns how many were changed.
#[tauri::command]
async fn set_window_mode_all(mode: prism::window::Mode) -> Result<usize, String> {
    let data = prism::data_dir();
    let mut n = 0;
    for inst in prism::list_instances(&data) {
        if let Some(g) = prism::game_dir(&data, &inst.id) {
            prism::window::set(&g, mode)?;
            n += 1;
        }
    }
    Ok(n)
}

/// Every top-level window: (handle, process id, visible, class, title).
fn top_windows() -> Vec<(isize, u32, bool, String, String)> {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumWindows(cb: unsafe extern "system" fn(isize, isize) -> i32, l: isize) -> i32;
        fn GetWindowTextW(h: isize, s: *mut u16, n: i32) -> i32;
        fn GetClassNameW(h: isize, s: *mut u16, n: i32) -> i32;
        fn IsWindowVisible(h: isize) -> i32;
        fn GetWindowThreadProcessId(h: isize, pid: *mut u32) -> u32;
    }
    unsafe extern "system" fn each(h: isize, l: isize) -> i32 {
        let out = unsafe { &mut *(l as *mut Vec<(isize, u32, bool, String, String)>) };
        let mut buf = [0u16; 256];
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(h, &mut pid);
            let n = GetWindowTextW(h, buf.as_mut_ptr(), 256).max(0) as usize;
            let title = String::from_utf16_lossy(&buf[..n]);
            let n = GetClassNameW(h, buf.as_mut_ptr(), 256).max(0) as usize;
            let class = String::from_utf16_lossy(&buf[..n]);
            out.push((h, pid, IsWindowVisible(h) != 0, class, title));
        }
        1
    }
    let mut out = Vec::new();
    unsafe { EnumWindows(each, &mut out as *mut _ as isize) };
    out
}

/// Visible Minecraft game windows (GLFW on modern versions, LWJGL on old ones).
fn game_windows() -> Vec<isize> {
    top_windows().into_iter()
        .filter(|(_, _, vis, class, title)| *vis && (class.starts_with("GLFW") || class == "LWJGL") && title.starts_with("Minecraft"))
        .map(|w| w.0)
        .collect()
}

/// Wait for the game window. Meanwhile Prism's progress box ("Please wait... - Prism Launcher x.y")
/// is read for the profile tile ("launch-progress" {id, pct, text}) and, when quiet, moved off-screen
/// and out of the taskbar; it stays "shown" there so its progress can still be read.
/// None if Prism closed or nothing appeared within 30 minutes (a first launch can download a lot).
/// The player pressed Cancel while their game was starting: the watcher stops and reports it.
static CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Play → Cancel: stop a game that's still starting (a misclick, or the wrong pack). Prism's download/prepare
/// step and the game's own process are both ended; nothing is lost, since no world is open yet.
#[tauri::command]
async fn cancel_launch(instance_id: String) -> Result<(), String> {
    CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
    blocking(move || {
        for (_, pid) in running_java().into_iter().filter(|(id, _)| *id == instance_id) {
            kill_tree(pid.as_u32());
        }
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        for (pid, _) in sys.processes().iter().filter(|(_, p)| p.name().eq_ignore_ascii_case("prismlauncher.exe")) {
            kill_tree(pid.as_u32());
        }
        Ok(())
    }).await
}

fn watch_launch(app: &tauri::AppHandle, id: &str, quiet: bool, before: &[isize]) -> Option<(u32, isize)> {
    use tauri::Emitter;
    let ui = uiautomation::UIAutomation::new().ok();
    let mut parked = std::collections::HashSet::new();
    let mut last = String::new();
    let mut sys = sysinfo::System::new();
    let mut prism: Vec<u32> = Vec::new();
    let started = std::time::Instant::now();
    let mut tick = 0u32;
    while started.elapsed() < std::time::Duration::from_secs(30 * 60) {
        if CANCEL.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
        if tick % 5 == 0 {
            sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
            prism = sys.processes().iter()
                .filter(|(_, p)| p.name().eq_ignore_ascii_case("prismlauncher.exe"))
                .map(|(pid, _)| pid.as_u32()).collect();
            // Give Prism a few seconds to start before deciding it's gone.
            if prism.is_empty() && started.elapsed() > std::time::Duration::from_secs(10) {
                return None;
            }
        }
        tick += 1;
        for (h, pid, vis, class, title) in top_windows() {
            if !vis {
                continue;
            }
            if (class.starts_with("GLFW") || class == "LWJGL") && title.starts_with("Minecraft") && !before.contains(&h) {
                return Some((pid, h));
            }
            if prism.contains(&pid) && title.starts_with("Please wait") {
                if quiet && parked.insert(h) {
                    park_offscreen(h);
                }
                if let Some((pct, text)) = ui.as_ref().filter(|_| tick % 3 == 0).and_then(|ui| prism_progress(ui, h)) {
                    let key = format!("{pct:?}{text}");
                    if key != last {
                        let _ = app.emit("launch-progress", serde_json::json!({ "id": id, "pct": pct.map(|p| (p * 100.0).round()), "text": text }));
                        last = key;
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    None
}

/// Move a window off-screen and out of the taskbar without hiding it.
fn park_offscreen(h: isize) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn ShowWindow(h: isize, cmd: i32) -> i32;
        fn GetWindowLongPtrW(h: isize, i: i32) -> isize;
        fn SetWindowLongPtrW(h: isize, i: i32, v: isize) -> isize;
        fn SetWindowPos(h: isize, after: isize, x: i32, y: i32, cx: i32, cy: i32, f: u32) -> i32;
    }
    const GWL_EXSTYLE: i32 = -20;
    unsafe {
        ShowWindow(h, 0 /* SW_HIDE */);
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        SetWindowLongPtrW(h, GWL_EXSTYLE, (ex | 0x80 /* TOOLWINDOW */) & !0x4_0000 /* APPWINDOW */);
        SetWindowPos(h, 0, -32000, -32000, 0, 0, 0x0001 | 0x0004 | 0x0010 /* NOSIZE | NOZORDER | NOACTIVATE */);
        ShowWindow(h, 4 /* SW_SHOWNOACTIVATE */);
    }
}

/// Prism's progress box through UI Automation (Qt exposes its widgets there): the overall bar as
/// 0..1 and a short line for the tile ("Getting game files · 165 of 402", else Prism's own text).
fn prism_progress(ui: &uiautomation::UIAutomation, hwnd: isize) -> Option<(Option<f64>, String)> {
    use uiautomation::patterns::UIRangeValuePattern;
    use uiautomation::types::{ControlType, TreeScope};
    let all = ui.element_from_handle(hwnd.into()).ok()?.find_all(TreeScope::Descendants, &ui.create_true_condition().ok()?).ok()?;
    let mut bar = None;
    let mut lines: Vec<String> = Vec::new();
    for e in all {
        match e.get_control_type() {
            Ok(ControlType::ProgressBar) if bar.is_none() => {
                if let Ok(p) = e.get_pattern::<UIRangeValuePattern>() {
                    let (lo, hi) = (p.get_minimum().unwrap_or(0.0), p.get_maximum().unwrap_or(0.0));
                    if let Ok(v) = p.get_value() {
                        if hi > lo { bar = Some(((v - lo) / (hi - lo)).clamp(0.0, 1.0)); }
                    }
                }
            }
            Ok(ControlType::Text) => {
                if let Ok(n) = e.get_name() {
                    let n = n.trim();
                    if !n.is_empty() && !n.starts_with("Ping") { lines.push(n.to_string()); }
                }
            }
            _ => {}
        }
    }
    let counted = lines.iter().find_map(|l| {
        let i = l.find(" out of ")?;
        let done = l[..i].rsplit(|c: char| !c.is_ascii_digit()).next().filter(|d| !d.is_empty())?;
        let total = l[i + 8..].split(|c: char| !c.is_ascii_digit()).next().filter(|d| !d.is_empty())?;
        Some(format!("Getting game files · {done} of {total}"))
    });
    let text = counted.or_else(|| lines.first().map(|l| l.chars().take(70).collect())).unwrap_or_else(|| "Preparing…".into());
    Some((bar, text))
}

/// The window mode every profile shares, or None when they differ.
#[tauri::command]
async fn window_mode_common() -> Result<Option<prism::window::Mode>, String> {
    let data = prism::data_dir();
    let modes: Vec<_> = prism::list_instances(&data).iter().filter_map(|i| prism::game_dir(&data, &i.id)).map(|g| prism::window::get(&g).mode).collect();
    Ok(modes.first().copied().filter(|m| modes.iter().all(|x| x == m)))
}

/// Running games by profile id: Java processes whose command line points into a Prism instance.
fn running_java() -> Vec<(String, sysinfo::Pid)> {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::All, true, sysinfo::ProcessRefreshKind::new().with_cmd(sysinfo::UpdateKind::Always));
    let mut out = Vec::new();
    for (pid, p) in sys.processes() {
        let name = p.name().to_string_lossy().to_ascii_lowercase();
        if !name.starts_with("java") {
            continue;
        }
        // Prism passes -Djava.library.path=<instances folder>/<id>/natives; that folder can be anywhere, in any case.
        if let Some(id) = p.cmd().iter().find_map(|a| prism::natives_instance(&a.to_string_lossy())) {
            out.push((id, *pid));
        }
    }
    out
}

/// A Prism that is already open keeps the pack folder it read at start-up, and a launch request goes to it. After
/// that setting was repaired, close it so the launch starts a Prism that reads the right folder. Never while a game
/// it started is still running.
async fn close_idle_prism() -> Result<(), String> {
    blocking(|| {
        if !running_java().is_empty() { return Ok(()) }
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        for p in sys.processes().values().filter(|p| p.name().eq_ignore_ascii_case("prismlauncher.exe")) {
            p.kill();
            p.wait();
        }
        Ok(())
    }).await
}

#[tauri::command]
async fn running_games() -> Vec<String> {
    blocking(|| Ok(running_java().into_iter().map(|(id, _)| id).collect())).await.unwrap_or_default()
}

#[derive(serde::Serialize)]
struct InstancesDir { dir: PathBuf, default: PathBuf }

/// Where the modpacks live (Prism's instances folder), and Prism's own default.
#[tauri::command]
async fn instances_dir_get() -> InstancesDir {
    let data = prism::data_dir();
    InstancesDir { dir: prism::instances_dir(&data), default: data.join("instances") }
}

/// Move every modpack to `dir` and point Prism there. Progress goes out as "instances-move"
/// events ({done, total} in bytes). Prism and every game must be closed: they hold the files open.
#[tauri::command]
async fn instances_dir_set(app: tauri::AppHandle, dir: String) -> Result<Vec<String>, String> {
    if prism_running().await { return Err("Close Prism Launcher first: it keeps the modpacks open.".into()) }
    if !running_java().is_empty() { return Err("Close every running game first.".into()) }
    let data = prism::data_dir();
    let to = PathBuf::from(dir);
    blocking(move || {
        use tauri::Emitter;
        let plan = prism::instdir::plan(&data, &to)?;
        // Only a copy to another drive needs room: a move on one drive is just a rename.
        let disks = sysinfo::Disks::new_with_refreshed_list();
        if let Some(d) = disks.list().iter().filter(|d| to.starts_with(d.mount_point())).max_by_key(|d| d.mount_point().as_os_str().len()) {
            if !plan.from.starts_with(d.mount_point()) && d.available_space() < plan.bytes + (1 << 30) {
                return Err(format!("{} has {} GB free; the modpacks need {} GB.", d.mount_point().display(), d.available_space() >> 30, (plan.bytes >> 30) + 1));
            }
        }
        let total = plan.bytes;
        let mut last = 0;
        let left = prism::instdir::run(&data, &plan, |done| {
            if done - last >= 64 << 20 || done == total {
                last = done;
                let _ = app.emit("instances-move", serde_json::json!({ "done": done, "total": total }));
            }
        })?;
        let _ = app.emit("instances-move", serde_json::json!({ "done": total, "total": total }));
        Ok(left)
    }).await
}

/// Force-close a frozen game (the Java process and anything it started).
#[tauri::command]
async fn kill_game(instance_id: String) -> Result<(), String> {
    blocking(move || {
        let pids: Vec<_> = running_java().into_iter().filter(|(id, _)| *id == instance_id).map(|(_, p)| p).collect();
        if pids.is_empty() {
            return Err("That game isn't running.".into());
        }
        for pid in pids {
            kill_tree(pid.as_u32());
        }
        Ok(())
    }).await
}

/// Bring the main window back (from the tray, or when Obelisk is started a second time).
fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// True when Obelisk runs inside another app's package (e.g. started from the Claude app): Windows
/// then redirects its AppData reads and writes, and Prism and Java started from it inherit that, which
/// makes them see stale copies of instances and load far slower.
#[tauri::command]
fn sandboxed() -> bool {
    static IT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *IT.get_or_init(|| {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentPackageFullName(len: *mut u32, name: *mut u16) -> i32;
        }
        let mut len = 0u32;
        // APPMODEL_ERROR_NO_PACKAGE (15700) means a normal desktop process.
        (unsafe { GetCurrentPackageFullName(&mut len, std::ptr::null_mut()) } != 15700) || appdata_redirected()
    })
}

/// A process started by a packaged app (the Claude app) can have no package identity of its own and still get its
/// AppData quietly redirected into that app's private copy (Packages\<app>\LocalCache): Obelisk then reads a stale
/// Prism config and shows the wrong modpacks. Seen by writing a file to AppData and finding it there.
fn appdata_redirected() -> bool {
    let (Some(roaming), Some(local)) = (std::env::var_os("APPDATA"), std::env::var_os("LOCALAPPDATA")) else { return false };
    let name = format!("obelisk-probe-{}", std::process::id());
    let probe = std::path::Path::new(&roaming).join(&name);
    if std::fs::write(&probe, b"").is_err() {
        return false;
    }
    let found = std::fs::read_dir(std::path::Path::new(&local).join("Packages")).into_iter().flatten().flatten()
        .any(|e| e.path().join("LocalCache").join("Roaming").join(&name).exists());
    let _ = std::fs::remove_file(&probe);
    found
}

/// Started inside another app's sandbox (see `sandboxed`): start again through Explorer, which runs Obelisk as a
/// normal desktop program, and let this copy end (true). Not twice in a row, should Explorer's start be redirected too.
pub fn leave_sandbox() -> bool {
    if !sandboxed() {
        return false;
    }
    let mark = std::env::temp_dir().join("obelisk-left-sandbox");
    let recent = std::fs::metadata(&mark).and_then(|m| m.modified()).ok()
        .and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() < 60);
    let Ok(exe) = std::env::current_exe() else { return false };
    if recent {
        return false;
    }
    let _ = std::fs::write(&mark, b"");
    std::process::Command::new("explorer.exe").arg(&exe).spawn().is_ok()
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Start Obelisk (in the tray) when Windows starts. Written directly (not by running reg.exe hidden,
/// which antivirus heuristics read as malware setting up persistence).
#[tauri::command]
fn set_autostart(on: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (key, _) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).create_subkey(RUN_KEY)
        .map_err(|e| format!("Windows didn't accept the start-up entry: {e}"))?;
    if on {
        key.set_value("Obelisk", &format!("\"{}\" --hidden", exe.display())).map_err(|e| format!("Windows didn't accept the start-up entry: {e}"))
    } else {
        let _ = key.delete_value("Obelisk");
        Ok(())
    }
}

/// End a process and everything it started (a game and its helpers), directly through the OS rather
/// than by running taskkill.exe, which antivirus heuristics count against unsigned programs.
/// True if the process was there to kill.
pub(crate) fn kill_tree(pid: u32) -> bool {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let root = sysinfo::Pid::from_u32(pid);
    if sys.process(root).is_none() { return false }
    let mut doomed = vec![root];
    let mut i = 0;
    while i < doomed.len() {
        let parent = doomed[i];
        doomed.extend(sys.processes().iter().filter(|(_, p)| p.parent() == Some(parent)).map(|(c, _)| *c));
        i += 1;
    }
    // Children first, so none is re-parented and missed.
    for p in doomed.iter().rev() {
        terminate(p.as_u32());
    }
    true
}

/// TerminateProcess (sysinfo's kill shells out to taskkill.exe).
fn terminate(pid: u32) {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn TerminateProcess(handle: isize, code: u32) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }
    const PROCESS_TERMINATE: u32 = 0x0001;
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if h != 0 {
            TerminateProcess(h, 1);
            CloseHandle(h);
        }
    }
}

/// `--after-pid N`: a restart (data folder move) waits for the old Obelisk to close first, up to a minute,
/// so the single-instance check doesn't hand over to it.
fn after_pid() {
    let args: Vec<String> = std::env::args().collect();
    let Some(pid) = args.iter().position(|a| a == "--after-pid").and_then(|i| args.get(i + 1)).and_then(|p| p.parse::<u32>().ok()) else { return };
    let t0 = std::time::Instant::now();
    let mut sys = sysinfo::System::new();
    while t0.elapsed() < std::time::Duration::from_secs(60) {
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]), true);
        if sys.process(sysinfo::Pid::from_u32(pid)).is_none() { return }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    after_pid();
    datadir::prepare();
    tauri::Builder::default()
        // A second start just brings the running Obelisk to the front.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // Repair pack and Java paths an older Obelisk wrote in a form Prism misreads (see prism::heal_cfg).
            std::thread::spawn(|| {
                let data = prism::data_dir();
                let _ = prism::heal_cfg(&data.join("prismlauncher.cfg"));
                if let Ok(rd) = std::fs::read_dir(prism::instances_dir(&data)) {
                    for e in rd.flatten() { let _ = prism::heal_cfg(&e.path().join("instance.cfg")); }
                }
            });
            // Slow jobs tell the window what they're doing (its small progress bar).
            {
                use tauri::Emitter;
                let h = app.handle().clone();
                prism::work::set_sink(move |w| { let _ = h.emit("work", w); });
            }
            // Tray icon: Obelisk keeps running there when the window is closed (Settings can turn that off).
            {
                use tauri::menu::{Menu, MenuItem};
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
                let open = MenuItem::with_id(app, "open", "Open Obelisk", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "Quit Obelisk", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&open, &quit])?;
                let mut tray = TrayIconBuilder::with_id("obelisk").tooltip("Obelisk").menu(&menu).show_menu_on_left_click(false)
                    .on_menu_event(|app, e| match e.id.as_ref() {
                        "open" => show_main(app),
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, e| {
                        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                            show_main(tray.app_handle());
                        }
                    });
                if let Some(icon) = app.default_window_icon() {
                    tray = tray.icon(icon.clone());
                }
                tray.build(app)?;
            }
            // Obelisk's window starts hidden. Started with Windows it stays in the tray until opened; otherwise the
            // small update window comes first (selfupdate.rs) and opens it.
            let hidden = std::env::args().any(|a| a == "--hidden");
            if !hidden && !selfupdate::resume_window(app.handle()) {
                selfupdate::start_window(app.handle());
            }
            // Local control API for the user's Minecraft chats (see control.rs).
            control::start(app.handle().clone());
            releases::start();
            cfaccount::start(app.handle().clone());
            if !sandboxed() {
                std::thread::spawn(mcp::refresh_helper);
            }
            selfupdate::watch(app.handle().clone(), hidden, || !control::busy() && running_java().is_empty());
            app.manage(std::sync::Arc::new(prism::modrinth::Client::new()?));
            // Shared packs: pull friends' changes every minute while Obelisk runs (only once a pack is shared).
            let net: ShareNet = Default::default();
            app.manage(net.clone());
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tauri::Emitter;
                // Blocked files still waiting for an answer (from before Obelisk was closed): ask once the window is up.
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                sweep_half_made().await;
                // Stores older Obelisk hard-linked jars into (once per pack); a shared pack resends what that broke.
                let shared = !prism::share::all(&prism::instances_dir(&prism::data_dir())).is_empty();
                let me = if shared { share_net(&handle, &net).await.ok().map(|n| n.member()) } else { None };
                let _ = blocking(move || { prism::share_net::heal_stores(&prism::instances_dir(&prism::data_dir()), me.as_ref()); Ok(()) }).await;
                for i in prism::list_instances(&prism::data_dir()) { announce_held(&handle, &i.id); }
                // Packs that send by themselves: the changes last seen, so they go out once they've stayed the same
                // for a whole minute (not halfway through an install), and never while that pack's game runs.
                let mut settling: HashMap<String, String> = HashMap::new();
                loop {
                    let packs = prism::share::all(&prism::instances_dir(&prism::data_dir()));
                    let any = packs.iter().any(|(_, _, s)| s.members.len() > 1);
                    if any {
                        if let Ok(n) = share_net(&handle, &net).await {
                            let running = running_games().await;
                            for (id, game, _) in packs.iter().filter(|(id, _, s)| s.auto_send && s.members.len() > 1 && !running.contains(id)) {
                                let g = game.clone();
                                let seen = blocking(move || prism::share::local_changes(&g)).await
                                    .ok().filter(|c| !c.is_empty()).and_then(|c| serde_json::to_string(&c).ok());
                                match seen {
                                    Some(now) if settling.get(id) == Some(&now) => {
                                        settling.remove(id);
                                        match n.publish(game, "Sent automatically") {
                                            Ok(Some(_)) => { let _ = handle.emit("share-changed", id); }
                                            Ok(None) => {}
                                            Err(e) => eprintln!("auto-send {id}: {e}"),
                                        }
                                    }
                                    Some(now) => { settling.insert(id.clone(), now); }
                                    None => { settling.remove(id); }
                                }
                            }
                            let synced = n.sync_all().await;
                            let got: usize = synced.iter().map(|(_, c)| c).sum();
                            if got > 0 {
                                let _ = handle.emit("share-incoming", got);
                            }
                            // Auto-applied changes may have brought blocked files.
                            for (id, _) in synced.iter().filter(|(_, c)| *c > 0) { announce_held(&handle, id); }
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                }
            });
            // Empty expired items out of every profile's trash.
            let handle = app.handle().clone();
            // Read every profile's mod jars once in the background, so opening a profile lists them instantly.
            std::thread::spawn(|| {
                let data = prism::data_dir();
                for inst in prism::list_instances(&data) {
                    if let Some(g) = prism::game_dir(&data, &inst.id) { prism::list_content(&g, "mods"); }
                }
            });
            std::thread::spawn(move || {
                let max = trash_ms(&handle);
                prism::packtrash::purge(&prism::instances_dir(&prism::data_dir()), pack_trash_ms(&handle), |p| trash::delete(p).map_err(|e| e.to_string()));
                for inst in prism::list_instances(&prism::data_dir()) {
                    if let Ok(g) = game(&inst.id) {
                        prism::trash::purge(&g, max);
                    }
                }
            });
            if let Some(w) = app.get_webview_window("main") {
                if let Ok(hwnd) = w.hwnd() {
                    no_accent_border(hwnd.0 as isize);
                    set_big_icon(hwnd.0 as isize);
                }
            }
            // Let the UI load icons, screenshots and world thumbnails from these folders only.
            let scope = app.asset_protocol_scope();
            scope.allow_directory(prism::instances_dir(&prism::data_dir()), true)?;
            scope.allow_directory(datadir::root().join("icons"), true)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let to_tray = settings_value(window.app_handle()).get("closeToTray").and_then(|v| v.as_bool()).unwrap_or(true);
                if to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            sandboxed, set_autostart, bridge::control_reply, mcp::ai_connect, selfupdate::self_update_check, selfupdate::self_update_install, selfupdate::startup_update, selfupdate::startup_done, selfupdate::whats_new, datadir::data_dir_get, datadir::data_dir_set,
            list_instances, instance_icon, set_instance_icon, list_content, watch::watch_content, content_icons, set_enabled, delete_content,
            list_worlds, list_screenshots, list_logs, read_log, reveal, launch,
            mr_search, mr_tags, mr_project, mr_versions, active_account, mr_user, mr_members, mr_installed, mr_install, import_pack, open_prism,
            detect_launchers, sync_sources, sync_profile, profile_options, set_profile_options, java_installs, java_at, java_download, duplicate_mods, ignore_duplicates, instances_dir_get, instances_dir_set, clone_instance,
            delete_instance, prism_defaults, set_prism_defaults, prism_running, system_memory_mb, get_settings, get_favorites, set_favorites,
            set_settings, set_cf_key, has_cf_key, mr_dependencies, cf_relations, cross_site, mr_token_set, mr_account, mr_follows, mr_follow, mr_notifications, mr_notifications_read, cfaccount::cf_account, cfaccount::cf_signin, cfaccount::cf_signout, cfaccount::cf_comments, cfaccount::cf_comment_post, cfaccount::cf_favorites, cfaccount::cf_favorite, cfaccount::cf_notifications, cfaccount::cf_notifications_read, list_accounts, switch_account, logout_account,
            check_updates, apply_update, await_download, cf_search, list_trash, restore_trash, remove_trash, crash_reports, diagnose_crash, read_report, lightning_runs, load_times, open_game_file, get_pins, set_pins, mod_configs, read_config, write_config, config_default, restore_config_default, orphan_configs, clean_configs, mod_sides, build_packs, reveal_pack, pack_trash, restore_pack, remove_pack, backups, create_checkpoint, auto_backup, change_backup, safety_backup, restore_backup, revert_change, share_status, share_overview, share_start, share_publish, share_sync, share_apply, share_apply_file, share_skip, share_set_auto, share_set_auto_send, releases_call, report_error, share_stop, share_remove, share_info, share_set_wishes, share_history, share_dismiss_ended, share_join, share_allow_held, held_list, held_keep, share_trace, collab_code, pack_code_start, pack_code_status, pack_code_stop, pack_code_preview, pack_code_cancel, pack_code_import, pack_export_zip, running_games, kill_game, cancel_launch, window_mode, set_window_mode, set_window_mode_all, window_mode_common, reveal_file, cf_project, cf_id_by_slug, cf_author, cf_files, cf_changelog, cf_install, cf_featured
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_, e| {
            if let tauri::RunEvent::Exit = e {
                control::stop();
            }
        });
}
