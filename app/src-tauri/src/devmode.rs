// Tester settings: crash logs, game logs, mod list and Obelisk errors go to each shared pack's owner.
// Opt-in; everything is scrubbed of tokens and the Windows user name before it leaves (core/src/devreport.rs).

use obelisk_core as prism;
use prism::devreport::{self, DevState, Report};
use std::path::PathBuf;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

fn file() -> PathBuf {
    super::datadir::root().join("devmode.json")
}

fn load() -> DevState {
    devreport::load(&file())
}

fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevView {
    unlocked: bool,
    crash: bool,
    logs: bool,
    mods: bool,
    client: bool,
    last: Option<String>,
}

fn view(s: &DevState) -> DevView {
    DevView { unlocked: s.unlocked(), crash: s.crash, logs: s.logs, mods: s.mods, client: s.client, last: s.last.clone() }
}

#[tauri::command]
pub async fn dev_unlock(pw: String) -> Result<DevView, String> {
    super::blocking(move || {
        let tag = devreport::unlock(&pw).ok_or("That's not it.")?;
        let _g = LOCK.lock().unwrap();
        let mut s = load();
        s.set_tag(tag);
        devreport::save(&file(), &s)?;
        Ok(view(&s))
    }).await
}

#[tauri::command]
pub async fn dev_state() -> DevView {
    view(&load())
}

#[tauri::command]
pub async fn dev_set(crash: bool, logs: bool, mods: bool, client: bool) -> Result<DevView, String> {
    let _g = LOCK.lock().unwrap();
    let mut s = load();
    if !s.unlocked() { return Err("Locked.".into()) }
    s.crash = crash;
    s.logs = logs;
    s.mods = mods;
    s.client = client;
    devreport::save(&file(), &s)?;
    Ok(view(&s))
}

#[tauri::command]
pub async fn dev_lock() -> Result<DevView, String> {
    let _g = LOCK.lock().unwrap();
    let mut s = load();
    s.lock();
    devreport::save(&file(), &s)?;
    Ok(view(&s))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sent {
    pack: String,
    ok: bool,
    by: Option<String>,
    ms: u64,
    error: Option<String>,
}

/// Packs shared with others that someone else owns: (id, folder, name).
fn tester_packs(me: &str) -> Vec<(String, PathBuf, String)> {
    prism::share::all(&prism::instances_dir(&prism::data_dir()))
        .into_iter()
        .filter(|(_, g, s)| s.members.len() > 1 && prism::share::owner(g, s) != me)
        .map(|(id, g, s)| (id, g, s.name))
        .collect()
}

/// Send a small ping to every owner and say whether it arrived.
#[tauri::command]
pub async fn dev_test_send(app: tauri::AppHandle, net: tauri::State<'_, super::ShareNet>) -> Result<Vec<Sent>, String> {
    if !load().unlocked() { return Err("Locked.".into()) }
    let n = super::share_net(&app, &net).await?;
    let packs = tester_packs(&n.member().id);
    if packs.is_empty() { return Err("No shared pack from someone else here yet: join one first.".into()) }
    let mut out = Vec::new();
    for (_, game, name) in packs {
        let r = Report::new("ping", "test", &name, version(), "test from Developer settings");
        let t = std::time::Instant::now();
        let res = n.report(&game, &r).await;
        let ms = t.elapsed().as_millis() as u64;
        out.push(match res {
            Ok(v) => Sent { pack: name, ok: true, by: v.get("by").and_then(|b| b.as_str()).map(String::from), ms, error: None },
            Err(e) => Sent { pack: name, ok: false, by: None, ms, error: Some(e) },
        });
    }
    let _g = LOCK.lock().unwrap();
    let mut s = load();
    let good = out.iter().filter(|x| x.ok).count();
    s.last = Some(format!("Test: {good} of {} reached", out.len()));
    let _ = devreport::save(&file(), &s);
    Ok(out)
}

/// One pass of the background sender (the share loop calls it every minute).
pub async fn tick(n: &prism::share_net::Net, running: &[String]) {
    let s = load();
    if !s.unlocked() || !s.any() { return }
    let me = n.member().id;
    let user = std::env::var("USERNAME").ok();
    let mut marks: Vec<(String, String)> = Vec::new();
    let mut last: Option<String> = None;
    let packs = tester_packs(&me);
    for (id, game, name) in &packs {
        let (g, nm, st, run) = (game.clone(), name.clone(), s.clone(), running.contains(id));
        let items = super::blocking(move || Ok(devreport::collect(&g, &nm, &st, run))).await.unwrap_or_default();
        for it in items {
            if it.kind.is_empty() { marks.push(it.mark); continue }
            let r = Report::new(it.kind, &it.name, name, version(), &devreport::scrub(&it.text, user.as_deref()));
            match n.report(game, &r).await {
                Ok(_) => { marks.push(it.mark); last = Some(format!("Sent {} {} ({name})", it.kind, it.name)); }
                Err(e) => { last = Some(format!("Couldn't send {} ({name}): {e}", it.kind)); break }
            }
        }
    }
    // Obelisk's own errors: once, to the first owner that takes them.
    if let Some(it) = devreport::collect_client(&super::datadir::root().join("errors.jsonl"), &s) {
        if it.kind.is_empty() {
            marks.push(it.mark);
        } else {
            let r = Report::new(it.kind, &it.name, "", version(), &devreport::scrub(&it.text, user.as_deref()));
            for (_, game, name) in &packs {
                if n.report(game, &r).await.is_ok() {
                    marks.push(it.mark.clone());
                    last = Some(format!("Sent Obelisk errors ({name})"));
                    break;
                }
            }
        }
    }
    if marks.is_empty() && last.is_none() { return }
    let _g = LOCK.lock().unwrap();
    let mut s = load();
    for (k, v) in marks { s.marks.insert(k, v); }
    if last.is_some() { s.last = last; }
    let _ = devreport::save(&file(), &s);
}
