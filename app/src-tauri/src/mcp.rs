//! `obelisk.exe --mcp`: an MCP server over stdio for AI assistants (Claude Code, Codex).
//!
//! It holds no power of its own: each tool call is forwarded to the running Obelisk's control API
//! (control.rs), which checks and logs it. The assistant gets nothing unless the user has switched on
//! Settings → Plugins → AI assistant.

use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};

fn copy_arg() -> Value {
    json!({ "type": "string", "description": "Test copy folder (see copies); \"play\" = the play instance (reads and mod changes only, backed up first, never launched)" })
}

// Kept short on purpose: every chat carries this list in its context on every turn (user, 2026-11-03).
pub(crate) fn tools() -> Value {
    let copy = copy_arg();
    let obj = |props: Value, req: &[&str]| json!({ "type": "object", "properties": props, "required": req });
    let t = |name: &str, desc: &str, schema: Value| json!({ "name": name, "description": desc, "inputSchema": schema });
    let st = || json!({ "type": "string" });
    let int = || json!({ "type": "integer" });
    let strs = || json!({ "type": "array", "items": { "type": "string" } });
    let job = || json!({ "job": { "type": "integer" } });
    json!([
        t("copies", "Test copies and whether a game runs in each.", obj(json!({}), &[])),
        t("launch", "Launch a test copy (waits for the bench lock; muted, off-screen, test harness). LIMITS: hold <=120 s (perf too); <=600 s only for a non-perf run whose purpose says \"interaction\"; timeout <=600; killed as \"overtime\" at 240 s + hold. Returns a job: use wait; end with kill when no save is needed.",
            obj(json!({ "copy": copy, "owner": st(), "purpose": st(),
                "preset": { "type": "string", "enum": ["crosspc"], "description": "Fixed cross-PC profiling run (perf)" },
                "eta": { "type": "integer", "description": "Minutes" },
                "world": { "type": "string", "description": "Save to open, or new:<seed>" },
                "new_world": { "type": "string", "description": "Seed for a fresh world, deleted after" },
                "test_world": { "type": "object", "description": "Custom world: layers [{block,height}] (bottom first; [] = void), terrain flat|normal, seed, biome, border, platform {y,size,block}, spawn {x,y,z}, hold, game_mode, setup [cmds], events [{at,every,command}] (seconds after arrival)" },
                "jvm": { "type": "array", "items": { "type": "string" }, "description": "-Dmod.key=value" },
                "timeout": int(),
                "camera": { "type": "object", "description": "{pitch,yaw} held for screenshots" },
                "jfr": { "type": "boolean" }, "jfr_roots": { "type": "boolean" },
                "at": { "type": "array", "items": { "type": "object" }, "description": "Hooks [{event,after,do}]; do: histogram|dump|threads|kill; event: a stage as in wait" },
                "disable_mods": strs(), "add_mods": strs(),
                "config": { "type": "array", "items": { "type": "object" }, "description": "[{file,key,value}] TOML for this run only" },
                "options": { "type": "object", "description": "options.txt for this run only" },
                "java": { "type": "integer", "description": "21 or 25; default the profile's" },
                "compact_headers": { "type": "boolean" },
                "gc": { "type": "array", "items": { "type": "string" }, "description": "One -XX:+Use...GC plus tuning, replaces the profile's" },
                "perf": { "type": "boolean", "description": "Bench to itself; only when the user asked" } }), &["copy", "owner", "purpose"])),
        t("gate", "Full-pack gate in the gate copy, optionally with a candidate jar. cycles <=2, hold <=120; overtime at 720 s.",
            obj(json!({ "owner": st(), "jar": st(), "replace": { "type": "string", "description": "Glob of the jar(s) it replaces" },
                "mods": st(), "cycles": int(), "hold": int(), "sync": { "type": "boolean" }, "tag": st(), "loader_version": st() }), &["owner"])),
        t("play_wait", "Wait until the user's own game is closed (timeout default 50 s); returns closed true/false.", obj(json!({ "timeout": int() }), &[])),
        t("status", "Obelisk jobs and their state.", obj(job(), &[])),
        t("wait", "Wait for a job to finish or reach a stage (until: window, modsLoaded, titleScreen, inWorld, terrainReady, leaveStart, quit, pressure0..3). timeout default 50, max 600. Samples: C:/mcw/obelisk-runs/<copy>/<job>.samples.jsonl.",
            obj(json!({ "job": { "type": "integer" }, "timeout": int(), "until": st() }), &["job"])),
        t("kill", "Fast quit a job's game, no save.", obj(job(), &["job"])),
        t("heap", "jcmd snapshot of a running launch into <copy>/jfr: histogram (default), dump (.hprof), threads (3 dumps, stuck frames by mod).",
            obj(json!({ "job": { "type": "integer" }, "kind": { "type": "string", "enum": ["histogram", "dump", "threads"] } }), &["job"])),
        t("mods", "A copy's mods, enabled state and duplicates.", obj(json!({ "copy": copy }), &["copy"])),
        t("mod_set", "Enable, disable or remove (moved aside, never deleted) a mod file in a copy.",
            obj(json!({ "copy": copy, "file": st(), "state": { "type": "string", "enum": ["enabled", "disabled", "removed"] } }), &["copy", "file", "state"])),
        t("mod_add", "Copy a bench .jar into a copy's mods (refused if its mod id is already there).", obj(json!({ "copy": copy, "jar": st() }), &["copy", "jar"])),
        t("mod_install", "Install from Modrinth (id/slug) or CurseForge (cf:<id>) with dependencies.",
            obj(json!({ "copy": st(), "project": st(), "version": st(), "loader": st(), "minecraft": st() }), &["copy", "project"])),
        t("updates", "Which files in a copy have updates, and which are on neither site.", obj(json!({ "copy": st(), "loader": st(), "minecraft": st() }), &["copy"])),
        t("mod_update", "Update one file to its newest fitting version.", obj(json!({ "copy": st(), "file": st(), "loader": st(), "minecraft": st() }), &["copy", "file"])),
        t("reports", "A copy's crash reports and logs.", obj(json!({ "copy": copy }), &["copy"])),
        t("diagnose", "Explain a crash report or log (default latest.log): cause, mods at fault, errors by mod.", obj(json!({ "copy": copy, "file": st() }), &["copy"])),
        t("log", "Read a log, grep (substring) and tail.", obj(json!({ "copy": copy, "file": st(), "grep": st(), "tail": int() }), &["copy"])),
        t("game_test", "Whole test in one call: launch, wait for the world, run steps, quit normally; returns stages, per-phase profile, step results, log summary, Lightning. preset basic: visit 4 structures. stress: tests [chests, mobs, hop, flyover]. dyno: Block Dyno's own plan (mode, default quick). Same limits as launch; stress/dyno measure <=120 s and end \"invalid\" if the user starts playing. background=true: follow with game.",
            obj(json!({ "copy": copy, "owner": st(), "purpose": st(),
                "preset": { "type": "string", "enum": ["basic", "stress", "dyno"] }, "mode": st(), "tests": strs(),
                "steps": { "type": "array", "items": { "type": "object" }, "description": "Extra steps (see game)" },
                "background": { "type": "boolean" }, "scenario_timeout": int(),
                "test_world": { "type": "object" }, "world": st(), "new_world": st(), "java": int(), "jvm": strs(), "options": { "type": "object" },
                "config": { "type": "array", "items": { "type": "object" } }, "add_mods": strs(), "disable_mods": strs(), "perf": { "type": "boolean" } }), &["copy", "owner", "purpose"])),
        t("game", "Talk to a running test game (job). op query: fps, mspt, heap, chunks, entities, player, errors. cmd: run command as the player. scenario: steps [{cmd},{tp:[x,y,z]},{wait:s},{wait_chunks:s},{sample:s},{locate:[ids]},{visit:{count,wait}},{hop},{flyover},{mobs},{chests},{assert:{field,min|max|equals}}]. quit: normal quit. stop: take over a game_test. result: game_test progress or result.",
            obj(json!({ "job": { "type": "integer" }, "op": { "type": "string", "enum": ["query", "cmd", "scenario", "quit", "stop", "result"] },
                "command": st(), "steps": { "type": "array", "items": { "type": "object" } }, "timeout": int() }), &["job", "op"])),
        t("screenshots", "Newest screenshot paths in a copy.", obj(json!({ "copy": copy, "count": int() }), &["copy"])),
        t("lightning", "Newest Lightning profiler runs in a copy, summarised, with results.json paths.", obj(json!({ "copy": copy, "count": int() }), &["copy"])),
        t("timeline", "Load times of a copy's newest sessions (launcher to quit).", obj(json!({ "copy": copy, "count": int() }), &["copy"])),
        t("app_commands", "Every command the Obelisk app's UI uses, with arguments. Run one with app.", obj(json!({}), &[])),
        t("app", "Run an Obelisk app command live, as if the user clicked it (args camelCase). Ask the user before anything big or hard to undo.",
            obj(json!({ "command": st(), "args": { "type": "object" }, "timeout": int() }), &["command"])),
        t("errors", "Errors Obelisk showed or hit, newest first.", obj(json!({ "limit": int(), "since": { "type": "integer", "description": "ms since 1970" } }), &[])),
        t("release_submit", "Submit a mod build to its release line. fix = stable: gated, then into every pack with the mod. feature = test build, Test Dummy only until promoted. Auto-rollback on a crash naming it.",
            obj(json!({ "jar": st(), "kind": { "type": "string", "enum": ["fix", "feature"] }, "owner": st(),
                "notes": st(), "mod": st(), "version": st() }), &["jar", "kind", "owner"])),
        t("release_status", "Release lines, versions per pack, rollout queue, installs, failures, rollbacks, notices.", obj(json!({}), &[])),
        t("release_promote", "Promote a test build to stable (rolls out to every pack).",
            obj(json!({ "mod": st(), "version": st(), "reason": { "type": "string", "enum": ["user-played", "proven-in-td"] } }), &["mod", "version", "reason"])),
        t("release_pack", "For one pack: rollback (put back the replaced jar), retry (queue a failed rollout again), optout or optin (automatic releases).",
            obj(json!({ "action": { "type": "string", "enum": ["rollback", "retry", "optout", "optin"] }, "mod": st(), "pack": st(), "reason": st() }), &["action", "pack"])),
        t("copy_pack", "New test copy from a copy or \"play\" (no saves unless saves=true).",
            obj(json!({ "from": copy, "to": st(), "saves": { "type": "boolean" }, "owner": st() }), &["from", "to"])),
    ])
}

/// The merged tools (game, mod_set, release_pack) to the control API's own commands.
fn route(name: &str, mut args: Value) -> Result<(String, Value), String> {
    let take = |args: &mut Value, k: &str| args.as_object_mut().and_then(|o| o.remove(k)).and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
    Ok(match name {
        "game" => {
            let op = take(&mut args, "op");
            if !["query", "cmd", "scenario", "quit", "stop", "result"].contains(&op.as_str()) { return Err("op: query, cmd, scenario, quit, stop or result".into()) }
            (format!("game-{op}"), args)
        }
        "mod-set" => {
            let cmd = match take(&mut args, "state").as_str() { "enabled" => "mod-enable", "disabled" => "mod-disable", "removed" => "mod-remove", _ => return Err("state: enabled, disabled or removed".into()) };
            (cmd.into(), args)
        }
        "release-pack" => match take(&mut args, "action").as_str() {
            "rollback" => ("release-rollback".into(), args),
            "retry" => ("release-retry".into(), args),
            a @ ("optout" | "optin") => { args["on"] = json!(a == "optout"); ("release-optout".into(), args) }
            _ => return Err("action: rollback, retry, optout or optin".into()),
        },
        _ => (name.to_string(), args),
    })
}

/// One request to the running Obelisk. Err = it isn't running or refused.
fn call(cmd: &str, args: &Value) -> Result<Value, String> {
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let info: Value = std::fs::read(std::path::Path::new(&home).join(".obelisk").join("control.json")).ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or("Obelisk isn't running. Ask the user to open Obelisk from the Start menu.")?;
    let port = info["port"].as_u64().ok_or("bad control.json")?;
    let token = info["token"].as_str().ok_or("bad control.json")?;
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port as u16))
        .map_err(|_| "Obelisk isn't running. Ask the user to open Obelisk from the Start menu.".to_string())?;
    let body = args.to_string();
    write!(s, "POST /v1/{cmd} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
        .map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n").ok_or("no answer from Obelisk")?;
    let ok = head.split_whitespace().nth(1) == Some("200");
    let v: Value = serde_json::from_str(body).unwrap_or(json!({ "error": body }));
    if ok { Ok(v) } else { Err(v["error"].as_str().map(String::from).unwrap_or_else(|| v.to_string())) }
}

/// The running Obelisk's tool list and version (an update brings new tools); this helper's own list when it isn't running.
fn live_tools() -> (Value, Option<String>) {
    match call("mcp-tools", &json!({})) {
        Ok(v) if v["tools"].is_array() => (v["tools"].clone(), v["version"].as_str().map(String::from)),
        _ => (tools(), None),
    }
}

pub fn serve() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    // The Obelisk version whose tools the assistant has; after a call, a different one means Obelisk updated meanwhile,
    // and the assistant is told to fetch the list again (tools/list_changed).
    let mut listed: Option<String> = None;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { continue }; // notifications need no answer
        let method = msg["method"].as_str().unwrap_or("");
        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": msg.pointer("/params/protocolVersion").cloned().unwrap_or(json!("2025-06-18")),
                "capabilities": { "tools": { "listChanged": true } },
                "serverInfo": { "name": "obelisk", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Obelisk runs Minecraft test copies in the test bench for you, outside your sandbox, so games load at normal speed. The play instance (\"play\") can be read (logs, diagnose, mods, copy_pack source) and its mods changed for experimental updates; each change takes a safety backup first, restorable in Obelisk's Backups. It is never launched: test in a copy. Every launch waits for the shared bench lock. Use wait instead of polling status."
            })),
            "ping" => Ok(json!({})),
            "tools/list" => {
                let (list, version) = live_tools();
                listed = version;
                Ok(json!({ "tools": list }))
            }
            "tools/call" => {
                let name = msg.pointer("/params/name").and_then(|n| n.as_str()).unwrap_or("").replace('_', "-");
                let args = msg.pointer("/params/arguments").cloned().unwrap_or(json!({}));
                let (name, mut args) = match route(&name, args) {
                    Ok(r) => r,
                    Err(e) => { let r = json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [{ "type": "text", "text": e }], "isError": true } });
                        if writeln!(out, "{r}").and_then(|_| out.flush()).is_err() { break } continue }
                };
                if (name == "wait" || name == "play-wait") && args.get("timeout").is_none() {
                    args["timeout"] = json!(50);
                }
                Ok(match call(&name, &args) {
                    Ok(v) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default() }] }),
                    Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
                })
            }
            _ => Err(json!({ "code": -32601, "message": format!("unknown method {method}") })),
        };
        let reply = match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
        };
        if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() {
            break;
        }
        if method == "tools/call" && listed.is_some() {
            let now = call("ping", &json!({})).ok().and_then(|v| v["version"].as_str().map(String::from));
            if now.is_some() && now != listed {
                listed = now;
                let note = json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" });
                if writeln!(out, "{note}").and_then(|_| out.flush()).is_err() { break; }
            }
        }
    }
}

/// Register `obelisk.exe --mcp` with the assistant (Settings → Plugins → Connect). Writes the assistant's
/// own config file directly: the Claude desktop app has no `claude` command on PATH.
#[tauri::command]
pub async fn ai_connect(assistant: String) -> Result<String, String> {
    // The helper program next to obelisk.exe (so connected chats never hold the app itself open), or the app.
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let helper = me.with_file_name("obelisk-mcp.exe");
    let exe = if helper.is_file() { helper } else { me }.to_string_lossy().into_owned();
    let home = std::path::PathBuf::from(std::env::var("USERPROFILE").map_err(|e| e.to_string())?);
    match assistant.as_str() {
        // Claude Code (also the desktop app's Code tab): user-scope servers live in ~/.claude.json. The desktop app's
        // own chats read claude_desktop_config.json instead: both get the entry.
        "claude" => {
            let code = add_server(&home.join(".claude.json"), json!({ "type": "stdio", "command": exe, "args": ["--mcp"], "env": {} }))?;
            let mut chat = false;
            for path in claude_desktop_configs() {
                chat |= add_server(&path, json!({ "command": exe, "args": ["--mcp"] }))?;
            }
            Ok(if code || chat { "Connected to Claude. Restart Claude (quit it from the tray too) and start a new chat." } else { "Already connected." }.into())
        }
        // Codex: ~/.codex/config.toml. A literal TOML string keeps the backslashes in the path.
        "gpt" => {
            let dir = home.join(".codex");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let path = dir.join("config.toml");
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if text.contains("[mcp_servers.obelisk]") {
                return Ok("Already connected.".into());
            }
            if exe.contains('\'') {
                return Err("Obelisk's install path has a quote in it; add it to Codex by hand".into());
            }
            let block = format!("\n[mcp_servers.obelisk]\ncommand = '{exe}'\nargs = [\"--mcp\"]\ntool_timeout_sec = 120\n");
            let sep = if text.is_empty() || text.ends_with('\n') { "" } else { "\n" };
            write_atomic(&path, format!("{text}{sep}{block}").as_bytes())?;
            Ok("Connected to Codex.".into())
        }
        _ => Err("unknown assistant".into()),
    }
}

/// Put the "obelisk" entry into an assistant's JSON config (a backup of the file first). False when it was already there.
fn add_server(path: &std::path::Path, entry: Value) -> Result<bool, String> {
    let shown = path.display();
    let mut cfg: Value = match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| format!("{shown} isn't valid JSON ({e}); left it alone"))?,
        Err(_) => json!({}),
    };
    if cfg.pointer("/mcpServers/obelisk") == Some(&entry) {
        return Ok(false);
    }
    if path.exists() {
        let mut bak = path.as_os_str().to_owned();
        bak.push(".obelisk-bak");
        std::fs::copy(path, bak).map_err(|e| e.to_string())?;
    } else if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let obj = cfg.as_object_mut().ok_or_else(|| format!("{shown} isn't a JSON object; left it alone"))?;
    let servers = obj.entry("mcpServers").or_insert_with(|| json!({}));
    servers.as_object_mut().ok_or_else(|| format!("mcpServers in {shown} isn't an object; left it alone"))?
        .insert("obelisk".into(), entry);
    write_atomic(path, &serde_json::to_vec_pretty(&cfg).map_err(|e| e.to_string())?)?;
    Ok(true)
}

/// The Claude desktop app's config for its chats: %APPDATA%\Claude, and the Store version's own copy of that folder
/// (it reads that one when it exists).
pub(crate) fn claude_desktop_configs() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(roaming) = std::env::var("APPDATA") {
        out.push(std::path::PathBuf::from(roaming).join("Claude").join("claude_desktop_config.json"));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let packages = std::path::PathBuf::from(local).join("Packages");
        for e in std::fs::read_dir(&packages).into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().starts_with("Claude_") {
                let dir = e.path().join("LocalCache").join("Roaming").join("Claude");
                if dir.is_dir() { out.push(dir.join("claude_desktop_config.json")) }
            }
        }
    }
    out
}

/// Write beside the file, then swap it in, so a crash never leaves half a config.
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("obelisk-tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// The assistants' MCP helper, obelisk-mcp.exe, is the same program under its own name, so connected chats never
/// hold obelisk.exe itself open (Windows can't replace a running program's file). The installer writes it; the
/// app never copies itself. At each start: sweep up copies an update renamed aside, and point Claude's entry at
/// the helper if it still runs this app (`obelisk.exe --mcp`) or an old install's helper.
pub fn refresh_helper() {
    let Ok(me) = std::env::current_exe() else { return };
    let Some(dir) = me.parent() else { return };
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if n.starts_with("obelisk-mcp.exe.old-") || n.starts_with("obelisk.exe.old") || n.starts_with("WebView2Loader.dll.old-") {
            let _ = std::fs::remove_file(e.path());
        }
    }
    let helper = dir.join("obelisk-mcp.exe");
    if helper.is_file() {
        let _ = repoint_claude(&helper);
    }
}

/// Claude's `obelisk` MCP entry, when it runs an Obelisk program other than `helper`, now runs `helper`.
/// Entries pointing anywhere else are someone's own choice and left alone. ~/.claude.json is backed up first.
fn repoint_claude(helper: &std::path::Path) -> Result<(), String> {
    let home = std::path::PathBuf::from(std::env::var("USERPROFILE").map_err(|e| e.to_string())?);
    let path = home.join(".claude.json");
    let Ok(bytes) = std::fs::read(&path) else { return Ok(()) };
    let mut cfg: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let want = helper.to_string_lossy().into_owned();
    let Some(cmd) = cfg.pointer("/mcpServers/obelisk/command").and_then(Value::as_str) else { return Ok(()) };
    let name = std::path::Path::new(cmd).file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if cmd.eq_ignore_ascii_case(&want) || !(name == "obelisk.exe" || name == "obelisk-mcp.exe") {
        return Ok(());
    }
    std::fs::copy(&path, home.join(".claude.json.obelisk-bak")).map_err(|e| e.to_string())?;
    cfg["mcpServers"]["obelisk"]["command"] = json!(want);
    cfg["mcpServers"]["obelisk"]["args"] = json!(["--mcp"]);
    write_atomic(&path, &serde_json::to_vec_pretty(&cfg).map_err(|e| e.to_string())?)
}
