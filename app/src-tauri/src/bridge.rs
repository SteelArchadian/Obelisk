//! The assistant's way to do anything the app can: the control API's `app` command runs one of the
//! app's own commands through the Obelisk window, exactly as a click would, so the window stays in step
//! (lists refresh, settings saved by the UI aren't overwritten). `app-commands` lists them with their
//! arguments and descriptions, read from this program's own source so the list never goes stale.
//! Account sign-outs, the CurseForge key, permanent deletes and letting in blocked files stay with the user.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, LazyLock, Mutex};
use std::time::Duration;
use tauri::Emitter;

const SOURCES: &[&str] = &[include_str!("lib.rs"), include_str!("datadir.rs"), include_str!("selfupdate.rs"), include_str!("mcp.rs")];

/// Never run for an assistant: secrets, accounts, deletes that can't be undone, and letting in a file Obelisk blocked.
const DENY: &[&str] = &["set_cf_key", "logout_account", "switch_account", "remove_pack", "remove_trash", "control_reply", "ai_connect", "share_allow_held"];

/// Handled by the window itself rather than a Rust command.
const UI: &[(&str, &str)] = &[
    ("ui.settings.get", "Obelisk's settings as the window holds them."),
    ("ui.settings.set", "Change settings: args is an object of the keys to change (see ui.settings.get); saved like the Settings page does."),
    ("ui.refresh", "Re-read the modpack list from disk."),
];

static PENDING: LazyLock<Mutex<HashMap<u64, mpsc::Sender<Result<Value, String>>>>> = LazyLock::new(Default::default);
static NEXT: AtomicU64 = AtomicU64::new(1);

/// Split a parameter list at top-level commas (types like `State<'_, Mr>` have their own).
fn params(list: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth) = (Vec::new(), String::new(), 0i32);
    for c in list.chars() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => { out.push(std::mem::take(&mut cur)); continue }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out.into_iter().map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|p| !p.is_empty()).collect()
}

fn camel(s: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in s.chars() {
        if c == '_' { up = true } else if up { out.extend(c.to_uppercase()); up = false } else { out.push(c) }
    }
    out
}

/// One command's arguments (as the window passes them: camelCase) and its doc comment.
fn describe(name: &str) -> Option<Value> {
    for src in SOURCES {
        let Some(at) = [format!("async fn {name}("), format!("fn {name}(")].iter().find_map(|p| src.find(p.as_str()).map(|i| i + p.len())) else { continue };
        let mut depth = 1;
        let end = src[at..].char_indices().find(|&(_, c)| { match c { '(' => depth += 1, ')' => depth -= 1, _ => {} } depth == 0 }).map(|(i, _)| at + i)?;
        let args: Vec<String> = params(&src[at..end]).into_iter()
            .filter(|p| !["AppHandle", "State<", "Window", "Webview"].iter().any(|t| p.contains(t)))
            .map(|p| match p.split_once(':') { Some((n, t)) => format!("{}: {}", camel(n.trim().trim_start_matches("mut ")), t.trim()), None => p })
            .collect();
        let head = &src[..src[..at].rfind('\n').unwrap_or(0)];
        let mut doc: Vec<&str> = Vec::new();
        for line in head.lines().rev() {
            let l = line.trim();
            if let Some(d) = l.strip_prefix("///") { doc.push(d.trim()) } else if l.starts_with("#[") { continue } else { break }
        }
        doc.reverse();
        return Some(json!({ "command": name, "args": args, "doc": doc.join(" ") }));
    }
    None
}

fn names() -> Vec<String> {
    let lib = SOURCES[0];
    let Some(start) = lib.find("generate_handler![").map(|i| i + "generate_handler![".len()) else { return vec![] };
    let end = lib[start..].find(']').map(|i| start + i).unwrap_or(start);
    lib[start..end].split(',').map(|n| n.trim().rsplit("::").next().unwrap_or("").to_string())
        .filter(|n| !n.is_empty() && !DENY.contains(&n.as_str())).collect()
}

pub fn commands() -> Value {
    let mut list: Vec<Value> = names().iter().map(|n| describe(n).unwrap_or_else(|| json!({ "command": n }))).collect();
    list.extend(UI.iter().map(|(n, d)| json!({ "command": n, "args": ["(see doc)"], "doc": d })));
    json!({ "commands": list, "note": "Run one with app {command, args}. Arguments are camelCase. Long ones (moving modpacks) may outlast timeout; they still finish, check with the matching _get command." })
}

/// Run `args.command` through the Obelisk window and return what it returned.
pub fn call(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let command = args.get("command").and_then(|v| v.as_str()).ok_or("missing command")?.to_string();
    if DENY.contains(&command.as_str()) {
        return Err(format!("{command} is for the user only (accounts, keys, permanent deletes and blocked files)."));
    }
    if !UI.iter().any(|(n, _)| *n == command) && !names().contains(&command) {
        return Err(format!("no command {command}; see app-commands"));
    }
    let timeout = args.get("timeout").and_then(|v| v.as_u64()).unwrap_or(120).clamp(5, 3600);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel();
    PENDING.lock().unwrap().insert(id, tx);
    app.emit("control-invoke", json!({ "id": id, "command": command, "args": args.get("args").cloned().unwrap_or(json!({})) }))
        .map_err(|e| e.to_string())?;
    let out = rx.recv_timeout(Duration::from_secs(timeout));
    PENDING.lock().unwrap().remove(&id);
    match out {
        Ok(r) => r.map(|v| json!({ "result": v })),
        Err(_) => Err(format!("{command} didn't answer within {timeout} s. It may still be running in the Obelisk window.")),
    }
}

/// The window's answer to a `control-invoke`.
#[tauri::command]
pub fn control_reply(id: u64, ok: bool, value: Value) {
    if let Some(tx) = PENDING.lock().unwrap().remove(&id) {
        let _ = tx.send(if ok { Ok(value) } else { Err(value.as_str().map(String::from).unwrap_or_else(|| value.to_string())) });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lists_commands_with_args() {
        let all = super::commands();
        let list = all["commands"].as_array().unwrap();
        let get = |n: &str| list.iter().find(|c| c["command"] == n).cloned();
        assert!(get("set_cf_key").is_none());
        let set = get("instances_dir_set").unwrap();
        assert_eq!(set["args"], serde_json::json!(["dir: String"]));
        assert!(set["doc"].as_str().unwrap().starts_with("Move every modpack"));
        assert_eq!(get("delete_content").unwrap()["args"][0], "instanceId: String");
    }
}
