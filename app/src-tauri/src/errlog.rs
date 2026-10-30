//! Every error Obelisk shows or hits in the background, kept in errors.jsonl in Obelisk's data folder so the
//! assistant can read them (the `errors` tool) and fix the causes. It stays on this PC; nothing is sent anywhere.

use serde_json::{json, Value};
use std::io::Write;
use std::sync::Mutex;

const MAX_BYTES: u64 = 1 << 20;

static LAST: Mutex<(String, u64)> = Mutex::new((String::new(), 0));

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn record(source: &str, message: &str) {
    let message: String = message.trim().chars().take(2000).collect();
    if message.is_empty() {
        return;
    }
    let t = now_ms();
    {
        // The same error again within a minute (a retry loop, a re-render) is one entry.
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if last.0 == message && t - last.1 < 60_000 {
            return;
        }
        *last = (message.clone(), t);
    }
    let dir = super::datadir::root();
    let path = dir.join("errors.jsonl");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&path, dir.join("errors.old.jsonl"));
    }
    let line = json!({ "time": t, "local": super::control::stamp(), "version": env!("CARGO_PKG_VERSION"), "source": source, "message": message });
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{line}");
    }
}

/// The newest errors, newest first; `since` (ms) leaves out older ones.
pub fn recent(limit: usize, since: u64) -> Value {
    let dir = super::datadir::root();
    let mut out: Vec<Value> = vec![];
    for name in ["errors.jsonl", "errors.old.jsonl"] {
        let text = std::fs::read_to_string(dir.join(name)).unwrap_or_default();
        out.extend(text.lines().rev().filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|v| v["time"].as_u64().unwrap_or(0) >= since));
        if out.len() >= limit {
            break;
        }
    }
    out.truncate(limit);
    json!({ "errors": out, "file": dir.join("errors.jsonl") })
}
