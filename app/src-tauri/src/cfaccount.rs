//! The user's CurseForge account, signed in on curseforge.com's own sign-in page in a small Obelisk window (Obelisk
//! never sees the password; the session lives in that window's cookies, shared by all of Obelisk's web views).
//! With it: comments and replies, favorites, and notifications of replies in threads the user commented in, checked
//! every 15 minutes (CurseForge has no notification feed of its own for this).

use obelisk_core::cfsite;
use serde_json::{json, Value};
use tauri::{Emitter, Manager};

const WINDOW: &str = "cf-signin";
const STORE: &str = "cf-watch.json";
const CHECK_MINS: u64 = 15;
/// Pages of a project's comments looked through for a watched thread (20 each, newest first).
const PAGES: u32 = 5;

/// The curseforge.com session cookies as a Cookie header ("" when signed out). Not on the main thread: reading
/// cookies there deadlocks WebView2.
fn cookie(app: &tauri::AppHandle) -> String {
    let Some(w) = app.get_webview_window(WINDOW).or_else(|| app.get_webview_window("main")) else { return String::new() };
    let url = cfsite::SITE.parse().expect("site url");
    w.cookies_for_url(url).unwrap_or_default().iter().map(|c| format!("{}={}", c.name(), c.value())).collect::<Vec<_>>().join("; ")
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

fn signed_in(app: &tauri::AppHandle) -> Result<(String, Value), String> {
    let c = cookie(app);
    if c.is_empty() {
        return Err("sign in to CurseForge first".into());
    }
    let me = cfsite::profile(&c)?.ok_or("sign in to CurseForge first")?;
    Ok((c, me))
}

/// The signed-in account, or null.
#[tauri::command]
pub async fn cf_account(app: tauri::AppHandle) -> Result<Value, String> {
    blocking(move || {
        let c = cookie(&app);
        if c.is_empty() { return Ok(Value::Null) }
        Ok(cfsite::profile(&c)?.unwrap_or(Value::Null))
    }).await
}

/// Open curseforge.com's sign-in page in a small window; it closes itself once the account is signed in
/// ("cf-account" event).
#[tauri::command]
pub async fn cf_signin(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(WINDOW) {
        // Already open (maybe behind something): bring it back.
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let url = format!("{0}/api/v1/auth/login?returnUrl={0}/&registrationCompletedUrl={0}/signup/confirmation", cfsite::SITE);
    let mut b = tauri::WebviewWindowBuilder::new(&app, WINDOW, tauri::WebviewUrl::External(url.parse().map_err(|e| format!("{e}"))?))
        .title("Sign in to CurseForge").inner_size(520.0, 760.0).center();
    // Owned by Obelisk's window, so clicking back into Obelisk doesn't bury it.
    if let Some(main) = app.get_webview_window("main") {
        b = b.parent(&main).map_err(|e| e.to_string())?;
    }
    b.build().map_err(|e| e.to_string())?;
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        // Whatever ends the sign-in, the page hears it ("cf-signin-ended"), so its button never stays stuck.
        for _ in 0..600 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if app2.get_webview_window(WINDOW).is_none() {
                let _ = app2.emit("cf-signin-ended", ());
                return; // closed by the user
            }
            let a = app2.clone();
            let me = blocking(move || { let c = cookie(&a); if c.is_empty() { Ok(None) } else { cfsite::profile(&c) } }).await;
            if let Ok(Some(me)) = me {
                if let Some(w) = app2.get_webview_window(WINDOW) { let _ = w.close(); }
                let _ = app2.emit("cf-account", me);
                return;
            }
        }
        if let Some(w) = app2.get_webview_window(WINDOW) { let _ = w.close(); }
        let _ = app2.emit("cf-signin-ended", ());
    });
    Ok(())
}

/// Sign out: the curseforge.com session cookies are removed.
#[tauri::command]
pub async fn cf_signout(app: tauri::AppHandle) -> Result<(), String> {
    blocking(move || {
        let w = app.get_webview_window("main").ok_or("no window")?;
        for site in [cfsite::SITE, "https://legacy.curseforge.com", "https://curseforge.com"] {
            for c in w.cookies_for_url(site.parse().expect("url")).unwrap_or_default() {
                let _ = w.delete_cookie(c);
            }
        }
        Ok(())
    }).await
}

/// A page of a project's comments ({comments, total, me}). Threads the user commented in are watched for replies.
#[tauri::command]
pub async fn cf_comments(app: tauri::AppHandle, project_id: u64, project_name: String, page: u32) -> Result<Value, String> {
    blocking(move || {
        let c = cookie(&app);
        let me = if c.is_empty() { None } else { cfsite::profile(&c).ok().flatten() };
        let mut v = cfsite::comments(project_id, page, Some(&c))?;
        if let Some(id) = me.as_ref().and_then(|m| m["id"].as_u64()) {
            let mut st = load();
            for t in v["comments"].as_array().into_iter().flatten() {
                let mine = t["author"]["id"].as_u64() == Some(id)
                    || t["replies"].as_array().into_iter().flatten().any(|r| r["author"]["id"].as_u64() == Some(id));
                if mine { watch(&mut st, project_id, &project_name, t) }
            }
            save(&st);
        }
        v["me"] = me.unwrap_or(Value::Null);
        Ok(v)
    }).await
}

/// Post a comment (or a reply under `parent_id`); its thread is watched for replies from then on.
#[tauri::command]
pub async fn cf_comment_post(app: tauri::AppHandle, project_id: u64, project_name: String, text: String, parent_id: Option<u64>) -> Result<(), String> {
    blocking(move || {
        let (c, me) = signed_in(&app)?;
        cfsite::post_comment(&c, project_id, &text, parent_id)?;
        // The new comment's thread: the parent's, or (a new comment) the newest of this user's on the first page.
        if let Ok(v) = cfsite::comments(project_id, 0, Some(&c)) {
            let mut st = load();
            let id = me["id"].as_u64();
            let thread = v["comments"].as_array().into_iter().flatten().find(|t| match parent_id {
                Some(p) => t["id"].as_u64() == Some(p),
                None => t["author"]["id"].as_u64() == id,
            });
            if let Some(t) = thread { watch(&mut st, project_id, &project_name, t) }
            save(&st);
        }
        Ok(())
    }).await
}

#[tauri::command]
pub async fn cf_favorites(app: tauri::AppHandle) -> Result<Vec<u64>, String> {
    blocking(move || { let (c, _) = signed_in(&app)?; cfsite::favorites(&c) }).await
}

#[tauri::command]
pub async fn cf_favorite(app: tauri::AppHandle, project_id: u64, on: bool) -> Result<(), String> {
    blocking(move || { let (c, _) = signed_in(&app)?; cfsite::set_favorite(&c, project_id, on) }).await
}

/// The account's CurseForge notifications, newest first: the site's own (approvals, status changes, comments, replies,
/// messages; see cfsite::notifications). When the site can't be asked, the replies Obelisk found in watched threads.
#[tauri::command]
pub async fn cf_notifications(app: tauri::AppHandle) -> Result<Value, String> {
    blocking(move || {
        let site = signed_in(&app).and_then(|(c, _)| cfsite::notifications(&c, 50));
        if let Ok(list) = site {
            return Ok(list.into());
        }
        let mut notes = load()["notes"].as_array().cloned().unwrap_or_default();
        notes.sort_by(|a, b| b["created"].as_u64().cmp(&a["created"].as_u64()));
        Ok(notes.into())
    }).await
}

/// Mark notifications read: the site's on curseforge.com (so its bell agrees), Obelisk's own here.
#[tauri::command]
pub async fn cf_notifications_read(app: tauri::AppHandle, ids: Vec<String>) -> Result<(), String> {
    blocking(move || {
        let site: Vec<u64> = ids.iter().filter_map(|i| i.strip_prefix("site-")?.parse().ok()).collect();
        if !site.is_empty() {
            let (c, _) = signed_in(&app)?;
            cfsite::notifications_read(&c, Some(site))?;
        }
        let mut st = load();
        for n in st["notes"].as_array_mut().into_iter().flatten() {
            if n["id"].as_str().is_some_and(|i| ids.iter().any(|x| x == i)) { n["read"] = true.into() }
        }
        save(&st);
        Ok(())
    }).await
}

// ---- watched threads ----

fn path() -> std::path::PathBuf {
    crate::datadir::root().join(STORE)
}

fn load() -> Value {
    std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({ "threads": [], "notes": [] }))
}

fn save(v: &Value) {
    let tmp = path().with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec_pretty(v).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, path());
    }
}

/// Start watching a thread (its replies so far count as seen).
fn watch(st: &mut Value, project_id: u64, project_name: &str, thread: &Value) {
    let Some(id) = thread["id"].as_u64() else { return };
    let threads = st["threads"].as_array_mut().expect("threads");
    if threads.iter().any(|t| t["threadId"].as_u64() == Some(id)) { return }
    let seen: Vec<Value> = thread["replies"].as_array().into_iter().flatten().map(|r| r["id"].clone()).collect();
    threads.push(json!({ "projectId": project_id, "projectName": project_name, "threadId": id, "seen": seen }));
}

/// Look through the watched threads for replies from someone else; each new one becomes a notification.
fn check(cookie: &str, me: u64) -> usize {
    let mut st = load();
    let mut found = 0;
    let projects: std::collections::BTreeSet<u64> = st["threads"].as_array().into_iter().flatten().filter_map(|t| t["projectId"].as_u64()).collect();
    for project in projects {
        let mut pages = vec![];
        for page in 0..PAGES {
            let Ok(v) = cfsite::comments(project, page, Some(cookie)) else { break };
            let list = v["comments"].as_array().cloned().unwrap_or_default();
            let done = list.len() < 20;
            pages.extend(list);
            let open = st["threads"].as_array().into_iter().flatten().filter(|t| t["projectId"].as_u64() == Some(project))
                .any(|t| !pages.iter().any(|c| c["id"] == t["threadId"]));
            if done || !open { break }
        }
        let mut notes = vec![];
        for t in st["threads"].as_array_mut().into_iter().flatten().filter(|t| t["projectId"].as_u64() == Some(project)) {
            let Some(thread) = pages.iter().find(|c| c["id"] == t["threadId"]) else { continue };
            for r in thread["replies"].as_array().into_iter().flatten() {
                if t["seen"].as_array().is_some_and(|s| s.contains(&r["id"])) { continue }
                t["seen"].as_array_mut().expect("seen").push(r["id"].clone());
                if r["author"]["id"].as_u64() == Some(me) { continue }
                let who = r["author"]["name"].as_str().or(r["author"]["username"].as_str()).unwrap_or("Someone");
                notes.push(json!({
                    "id": format!("cf-{}", r["id"]), "title": format!("{who} replied on {}", t["projectName"].as_str().unwrap_or("a project")),
                    "text": plain(r["html"].as_str().unwrap_or_default()), "projectId": project, "commentId": t["threadId"],
                    "read": false, "created": r["posted"],
                }));
            }
        }
        found += notes.len();
        st["notes"].as_array_mut().expect("notes").extend(notes);
    }
    if let Some(n) = st["notes"].as_array_mut() {
        n.sort_by(|a, b| b["created"].as_u64().cmp(&a["created"].as_u64()));
        n.truncate(200);
    }
    save(&st);
    found
}

/// A reply's text without its HTML, shortened for the bell.
fn plain(html: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for ch in html.chars() {
        match ch { '<' => tag = true, '>' => { tag = false; out.push(' ') } _ if !tag => out.push(ch), _ => {} }
    }
    let t = out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'");
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > 160 { format!("{}…", t.chars().take(160).collect::<String>()) } else { t }
}

/// Every 15 minutes while signed in: when the site counts more new notifications than last time, or a watched thread
/// has a new reply, the bell reloads ("cf-notes" event).
pub fn start(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last = 0u64;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let a = app.clone();
            let (count, replies) = blocking(move || {
                let (c, me) = signed_in(&a)?;
                let count = cfsite::notifications_count(&c).unwrap_or(0);
                let watching = load()["threads"].as_array().is_some_and(|t| !t.is_empty());
                Ok((count, if watching { check(&c, me["id"].as_u64().unwrap_or(0)) } else { 0 }))
            }).await.unwrap_or((0, 0));
            if count > last || replies > 0 { let _ = app.emit("cf-notes", count.max(replies as u64)); }
            last = count;
            tokio::time::sleep(std::time::Duration::from_secs(CHECK_MINS * 60 - 60)).await;
        }
    });
}
