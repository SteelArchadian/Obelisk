//! The notifications on legacy.curseforge.com/my-notifications: project and file approvals, status changes with
//! moderation's notes, comments on the user's projects. curseforge.com's own feed (cfsite::notifications) has only
//! some of them. The page sits behind Cloudflare's browser check, so it's read the way the user would: loaded in a
//! hidden Obelisk web view (their own CurseForge session), with a small script that reads the list off the page and
//! hands it back by navigating to an address Obelisk catches. Nothing is clicked or filled in.

use obelisk_core::cfsite;
use serde_json::{json, Value};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

const WINDOW: &str = "cf-legacy";
const PAGE: &str = "https://legacy.curseforge.com/my-notifications";
/// The address the script "navigates" to with what it read; Obelisk cancels the navigation and takes the data.
const BACK: &str = "obelisk.invalid";
/// How old the list may get before the bell asks the page again.
const STALE_SECS: u64 = 600;
const TIMEOUT_SECS: u64 = 60;

/// What the page showed last: {at, items, signedIn, error}.
static LAST: Mutex<Option<Value>> = Mutex::new(None);

const SCRIPT: &str = r#"
(() => {
  if (location.hostname !== "legacy.curseforge.com") return;
  const send = (d) => { location.href = "https://obelisk.invalid/#" + encodeURIComponent(JSON.stringify(d)); };
  let tries = 0;
  const read = () => {
    tries++;
    if (/just a moment|attention required/i.test(document.title)) { if (tries < 40) setTimeout(read, 1000); return; }
    if (!location.pathname.startsWith("/my-notifications")) { send({ signedIn: false, items: [], path: location.pathname }); return; }
    const main = document.querySelector("main, #content, .content, .main-content, body");
    const seen = new Set();
    const items = [];
    for (const d of main.querySelectorAll("[data-epoch]")) {
      if (d.closest("header, footer, nav")) continue;
      const box = d.closest("li, tr, article, [class*='notification']") || d.parentElement?.parentElement || d.parentElement;
      if (!box || seen.has(box)) continue;
      seen.add(box);
      const copy = box.cloneNode(true);
      copy.querySelectorAll("[data-epoch], script, style, button").forEach((e) => e.remove());
      const lines = (copy.innerText || copy.textContent || "").split("\n").map((l) => l.replace(/\s+/g, " ").trim()).filter(Boolean);
      items.push({
        epoch: Number(d.getAttribute("data-epoch")) || 0,
        lines,
        unread: /\b(unread|new|is-new|not-read)\b/i.test(box.className + " " + (box.parentElement?.className || "")),
        links: [...box.querySelectorAll("a[href]")].map((a) => a.href),
      });
    }
    send({ signedIn: !document.querySelector("a[href*='/login']") || items.length > 0, items, diag: (main.innerHTML || "").slice(0, 60000) });
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", () => setTimeout(read, 300));
  else setTimeout(read, 300);
})();
"#;

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The last list read off the page (empty until the first read), and whether it's time to read it again.
pub fn cached() -> (Vec<Value>, bool) {
    let last = LAST.lock().expect("legacy").clone();
    let stale = last.as_ref().is_none_or(|l| now().saturating_sub(l["at"].as_u64().unwrap_or(0)) > STALE_SECS);
    (last.map(|l| l["items"].as_array().cloned().unwrap_or_default()).unwrap_or_default(), stale)
}

/// Mark legacy items read in Obelisk (the legacy site marks them read itself when its page is opened).
pub fn mark_read(ids: &[String]) {
    if let Some(l) = LAST.lock().expect("legacy").as_mut() {
        for n in l["items"].as_array_mut().into_iter().flatten() {
            if n["id"].as_str().is_some_and(|i| ids.iter().any(|x| x == i)) { n["read"] = true.into() }
        }
    }
}

/// Read the page again in the background (once at a time); "cf-notes" goes out when the list is in.
pub fn refresh(app: &tauri::AppHandle, read: Vec<String>) {
    if app.get_webview_window(WINDOW).is_some() {
        return;
    }
    let a = app.clone();
    let built = tauri::WebviewWindowBuilder::new(app, WINDOW, tauri::WebviewUrl::External(PAGE.parse().expect("page url")))
        .title("CurseForge notifications").visible(false).skip_taskbar(true).initialization_script(SCRIPT)
        .on_navigation(move |url| {
            if url.host_str() != Some(BACK) {
                return true;
            }
            let data = serde_json::from_str::<Value>(&cfsite::decode(url.fragment().unwrap_or_default())).unwrap_or(Value::Null);
            take(&data, &read);
            let a = a.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(w) = a.get_webview_window(WINDOW) { let _ = w.close(); }
                let _ = a.emit("cf-notes", 0);
            });
            false
        })
        .build();
    if built.is_err() {
        return;
    }
    let a = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(TIMEOUT_SECS)).await;
        if let Some(w) = a.get_webview_window(WINDOW) {
            let _ = w.close();
            let mut last = LAST.lock().expect("legacy");
            let items = last.as_ref().map(|l| l["items"].clone()).unwrap_or(json!([]));
            *last = Some(json!({ "at": now(), "items": items, "error": "legacy.curseforge.com didn't load" }));
        }
    });
}

/// Keep what the page showed (and the page itself, for working out a page CurseForge changed).
fn take(data: &Value, read: &[String]) {
    if let Some(diag) = data["diag"].as_str() {
        let _ = std::fs::write(crate::datadir::root().join("cf-legacy-page.html"), diag);
    }
    let items: Vec<Value> = data["items"].as_array().into_iter().flatten().filter_map(|i| cfsite::legacy_item(i, read)).collect();
    *LAST.lock().expect("legacy") = Some(json!({ "at": now(), "items": items, "signedIn": data["signedIn"] }));
}
