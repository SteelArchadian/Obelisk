//! `obelisk.exe --mcp`: an MCP server over stdio for AI assistants (Claude Code, Codex).
//!
//! It holds no power of its own: each tool call is forwarded to the running Obelisk's control API
//! (control.rs), which checks and logs it. The assistant gets nothing unless the user has switched on
//! Settings → Plugins → AI assistant.

use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};

fn copy_arg() -> Value {
    json!({ "type": "string", "description": "A test copy folder directly in the test bench (see copies). Read-only tools and the mod tools also take \"play\" for the play instance (mod changes there take a safety backup first; it is never launched)." })
}

pub(crate) fn tools() -> Value {
    let copy = copy_arg();
    let obj = |props: Value, req: &[&str]| json!({ "type": "object", "properties": props, "required": req });
    let t = |name: &str, desc: &str, schema: Value| json!({ "name": name, "description": desc, "inputSchema": schema });
    json!([
        t("copies", "List the test copies in the test bench and whether a game runs in each.", obj(json!({}), &[])),
        t("launch", "Launch Minecraft in a test copy (never the play instance), at full speed outside the assistant's sandbox. Waits for the bench lock first (shared with other test runs, one per copy, unless perf is set), mutes the game, adds the test harness (no mouse grab), turns off Absolute AFK Cinema, parks the window off-screen, and kills it if still alive 30 s after \"Stopping!\". Returns a job id; use wait. End runs that need no saved progress with kill (fast quit).",
            obj(json!({ "copy": copy, "owner": { "type": "string", "description": "Your chat's name (for the bench lock)" },
                "purpose": { "type": "string" }, "eta": { "type": "integer", "description": "Expected minutes, 1-30" },
                "world": { "type": "string", "description": "Save folder to open automatically (Speed Loader autobench), or \"new:<seed>\" for a fresh world (same as new_world)" },
                "jvm": { "type": "array", "items": { "type": "string" }, "description": "Extra -Dmod.key=value properties, e.g. -Dlightning.capture=gpu,cpu or -Dpackbench.scene=gate" },
                "camera": { "type": "object", "description": "Hold the player's view for aimed screenshots (sky, shadows, models). pitch: -90 straight up to 90 straight down; yaw: 0 south, 90 west, 180 north, -90 east. Either may be left out",
                    "properties": { "pitch": { "type": "number" }, "yaw": { "type": "number" } } },
                "timeout": { "type": "integer", "description": "Seconds before the run counts as a hang (60-3600, default 900)" },
                "park": { "type": "boolean", "description": "Ignored: test windows always stay off-screen in the background" },
                "new_world": { "type": "string", "description": "Seed: Speed Loader creates and benches a new world; it is deleted after the run" },
                "test_world": { "type": "object", "description": "Build a custom world for this run and open it (instead of world/new_world). Overworld is flat with your layers (none = empty void); nether and end are normal. The previous test world in the copy is replaced. Scripted events count from when the player is first in the world; commands run as the server (no leading slash needed). The save is saves/obtest-<time>",
                    "properties": {
                        "layers": { "type": "array", "description": "Bottom first from y=-64, e.g. [{block:\"bedrock\",height:1},{block:\"sand\",height:3}]. Default bedrock, 2 dirt, grass. [] = void",
                            "items": { "type": "object", "properties": { "block": { "type": "string" }, "height": { "type": "integer" } } } },
                        "terrain": { "type": "string", "enum": ["flat", "normal"], "description": "normal: real overworld terrain (hills, water, caves, structures) from seed, the player starting at y 200 (use game_mode spectator to fly with tp events). Default flat" },
                        "seed": { "description": "With terrain normal: a number or text, as in Minecraft's create-world screen (default 0)" },
                        "biome": { "type": "string", "description": "Default plains (the_void when layers is [])" },
                        "border": { "type": "integer", "description": "World border width in blocks (16-60000000), centred on the spawn" },
                        "platform": { "type": "object", "description": "A square slab built when the world starts, e.g. a sky island: {y:150,size:32,block:\"stone\"} (size up to 1024). The player starts on it",
                            "properties": { "y": { "type": "integer" }, "size": { "type": "integer" }, "block": { "type": "string" } } },
                        "spawn": { "type": "object", "description": "Where the player starts {x,y,z}; default on the platform or the top layer at 0,0",
                            "properties": { "x": { "type": "integer" }, "y": { "type": "integer" }, "z": { "type": "integer" } } },
                        "hold": { "type": "integer", "description": "Seconds to stay in the world before the run quits (5-3600). Default: long enough for the last event (+60 s with repeating events), at least 20" },
                        "game_mode": { "type": "string", "enum": ["creative", "survival", "adventure", "spectator"], "description": "Default creative" },
                        "setup": { "type": "array", "items": { "type": "string" }, "description": "Commands run once when the world first starts, e.g. \"gamerule doDaylightCycle false\", \"time set night\"" },
                        "events": { "type": "array", "description": "Scripted commands: at seconds after the player arrives, optionally repeating every N seconds, e.g. {at:5,command:\"summon zombie 4 151 4\"}",
                            "items": { "type": "object", "properties": { "at": { "type": "number" }, "every": { "type": "number" }, "command": { "type": "string" } } } } } },
                "jfr": { "type": "boolean", "description": "Record a JFR profile to <copy>/jfr/obelisk-<time>.jfr (CPU samples, allocation samples with stacks over time, GC and heap summaries, old-object samples)" },
                "at": { "type": "array", "description": "Hooks at stages: when event has happened and after more seconds passed, do it. Results land in the job's hooks (wait shows them)",
                    "items": { "type": "object", "properties": {
                        "event": { "type": "string", "description": "window, modsLoaded, resourcesLoaded, titleScreen, joinStart, inWorld, terrainReady, leaveStart, leaveDone, quit, pressure0..3" },
                        "after": { "type": "number", "description": "Seconds after the event (default 0)" },
                        "do": { "type": "string", "enum": ["histogram", "dump", "threads", "kill"], "description": "histogram/dump/threads: snapshot to <copy>/jfr (see heap); kill: fast quit there (ends the game at once, no world save; the quickest way to finish a run that needs no saved progress)" } },
                        "required": ["event", "do"] } },
                "jfr_roots": { "type": "boolean", "description": "With jfr: keep each old-object sample's reference chain to its GC root (who holds it); adds time when the recording is written at exit" },
                "disable_mods": { "type": "array", "items": { "type": "string" }, "description": "Jar file names in mods/ to set aside for this run only" },
                "add_mods": { "type": "array", "items": { "type": "string" }, "description": "Jars in the test bench to add for this run only" },
                "config": { "type": "array", "items": { "type": "object", "properties": { "file": { "type": "string" }, "key": { "type": "string" }, "value": { "type": "string" } }, "required": ["file", "key", "value"] }, "description": "TOML settings for this run only: file relative to the copy, dotted key (section.key, must exist), value as TOML (true, 12, \"text\"). Restored byte-for-byte after" },
                "java": { "type": "integer", "description": "Leave out to use the Java of the profile the copy came from (its JVM args and memory are always used). 21 = the install's Java, 25 = jdk-25* in the test bench's java folder" },
                "compact_headers": { "type": "boolean", "description": "Java 25 only: -XX:+UseCompactObjectHeaders" },
                "gc": { "type": "array", "items": { "type": "string" }, "description": "This run's garbage collector, in place of every GC flag from the profile and bench defaults (nothing saved): exactly one -XX:+Use...GC plus GC tuning only, e.g. [\"-XX:+UseShenandoahGC\", \"-XX:ShenandoahGCMode=generational\", \"-XX:ConcGCThreads=2\"]. The flags used appear in the launch reply and the job result." },
                "options": { "type": "object", "description": "options.txt settings for this run only, put back after it, e.g. {\"renderDistance\": 12, \"simulationDistance\": 8}. Test copies run at renderDistance 6 unless a run sets it." },
                "perf": { "type": "boolean", "description": "A performance test (FPS, load times, memory): waits until no other test runs and has the bench to itself. Only when the user asked for one. Default false: shares the bench with other non-performance runs, one run per copy" } }), &["copy", "owner", "purpose"])),
        t("gate", "Run the full-pack gate (gate.py in the test bench's scripts) in its gate copy, optionally swapping in a candidate jar.",
            obj(json!({ "owner": { "type": "string" }, "jar": { "type": "string", "description": "Candidate .jar in the test bench" },
                "replace": { "type": "string", "description": "Glob in mods/ of the jar(s) it replaces" }, "mods": { "type": "string" },
                "cycles": { "type": "integer" }, "hold": { "type": "integer" }, "sync": { "type": "boolean" }, "tag": { "type": "string" },
                "loader_version": { "type": "string", "description": "A loader version folder in the install to run instead of the default (e.g. a library-override test version like neoforge-21.1.252-mixin017)" } }), &["owner"])),
        t("status", "Jobs started through Obelisk (launches, gates, copies) and their state.", obj(json!({ "job": { "type": "integer" } }), &[])),
        t("wait", "Block until a job finishes, reaches a stage (until), or the timeout passes (default 50 s, max 600; keep it under your client's tool timeout), then return the job. Call again while it is still queued or running. A launch's job carries events (stages with seconds since Java started) and now (latest CPU/memory/GPU/heap sample); every sample is in C:/mcw/obelisk-runs/<copy>/<job>.samples.jsonl.",
            obj(json!({ "job": { "type": "integer" }, "timeout": { "type": "integer" },
                "until": { "type": "string", "description": "Return as soon as this stage happens: window, modsLoaded, resourcesLoaded, titleScreen, joinStart, inWorld, terrainReady, leaveStart, leaveDone, quit, or pressure0..pressure3 (Optimal's memory pressure)" } }), &["job"])),
        t("kill", "Fast quit: ends a job's game (or script) at once without saving, the same as End task. Use it to finish any test that doesn't need its world saved, instead of waiting for a normal quit. In the game itself, shift-clicking the pause menu's quit button does the same.", obj(json!({ "job": { "type": "integer" } }), &["job"])),
        t("heap", "Snapshot a running launch's heap with jcmd, into <copy>/jfr/. \"histogram\" (default): live objects per class, count and shallow bytes; forces a full GC, a few seconds, repeat it for a series over time. \"dump\": an .hprof of live objects for retained sizes in Eclipse MAT or VisualVM; freezes the game while writing and is about as large as the used heap. \"threads\": where a hang is: three thread dumps 1.5 s apart; for the render, server and main threads and busy loading workers, their top frames, whether they stayed stuck on the same frames, and which packages (mods) they are in.",
            obj(json!({ "job": { "type": "integer" }, "kind": { "type": "string", "enum": ["histogram", "dump", "threads"] } }), &["job"])),
        t("mods", "List a copy's mods with file names and enabled state, plus any duplicates (same mod installed twice).", obj(json!({ "copy": copy }), &["copy"])),
        t("mod_enable", "Enable a mod in a test copy.", obj(json!({ "copy": copy, "file": { "type": "string" } }), &["copy", "file"])),
        t("mod_disable", "Disable a mod in a test copy.", obj(json!({ "copy": copy, "file": { "type": "string" } }), &["copy", "file"])),
        t("mod_add", "Copy a .jar from in the test bench into a test copy's mods. Refused if another enabled jar already provides the same mod id.", obj(json!({ "copy": copy, "jar": { "type": "string" } }), &["copy", "jar"])),
        t("mod_remove", "Move a mod out of a test copy (into mods-removed-obelisk; never deleted).", obj(json!({ "copy": copy, "file": { "type": "string" } }), &["copy", "file"])),
        t("mod_install", "Install a mod (and its required dependencies) from Modrinth or CurseForge into a copy or play (play gets a safety backup first). Refuses a mod that is already there from the other site. Loader/Minecraft default to Foundation's.",
            obj(json!({ "copy": { "type": "string" }, "project": { "type": "string", "description": "Modrinth project id or slug, or cf:<CurseForge project id>" },
                "version": { "type": "string", "description": "Exact Modrinth version id or CurseForge file id (default: newest that fits)" },
                "loader": { "type": "string" }, "minecraft": { "type": "string" } }), &["copy", "project"])),
        t("updates", "Which files in a copy (or play) have updates on Modrinth/CurseForge, and which are on neither site.",
            obj(json!({ "copy": { "type": "string" }, "loader": { "type": "string" }, "minecraft": { "type": "string" } }), &["copy"])),
        t("mod_update", "Update one file to its newest fitting version (play gets a safety backup first).",
            obj(json!({ "copy": { "type": "string" }, "file": { "type": "string", "description": "File name from mods/updates" },
                "loader": { "type": "string" }, "minecraft": { "type": "string" } }), &["copy", "file"])),
        t("duplicates", "Mods present twice among a copy's enabled jars (same mod id, usually one from each site); the game won't start until one is disabled. mods also lists these.",
            obj(json!({ "copy": { "type": "string" } }), &["copy"])),
        t("reports", "List a copy's crash reports and logs.", obj(json!({ "copy": copy }), &["copy"])),
        t("diagnose", "Explain a crash report or log: cause, mods at fault, slow loading by mod, errors by mod.", obj(json!({ "copy": copy, "file": { "type": "string", "description": "Default logs/latest.log" } }), &["copy"])),
        t("log", "Read a log, optionally filtered (case-insensitive substring) and tailed.", obj(json!({ "copy": copy, "file": { "type": "string" }, "grep": { "type": "string" }, "tail": { "type": "integer" } }), &["copy"])),
        t("screenshots", "Newest screenshot paths in a copy.", obj(json!({ "copy": copy, "count": { "type": "integer" } }), &["copy"])),
        t("lightning", "Newest Lightning profiler runs in a copy, summarised as in the Diagnosis tab (FPS, frame times, 1% low, GPU-bound %, tick times, heap and GC), plus their results.json paths for per-mod detail.", obj(json!({ "copy": copy, "count": { "type": "integer" } }), &["copy"])),
        t("timeline", "Load times of the newest game sessions in a copy (or play): launcher, Java, mods, resources to the menu, each world join/leave and quitting, from Obelisk's view plus the log and Optimal's (formerly Speed Loader) bench.jsonl. Saved per session in <copy>/obelisk-timeline.jsonl.",
            obj(json!({ "copy": copy, "count": { "type": "integer" } }), &["copy"])),
        t("app_commands", "Everything the Obelisk app can do (each command its UI uses), with arguments and descriptions: modpacks, mods, settings, Java, backups, updates, sharing, data and modpack folders. Run one with app.", obj(json!({}), &[])),
        t("app", "Run one Obelisk command live through its window, as if the user clicked it (lists and settings in the window stay in step). See app_commands for names and arguments (camelCase). Changes show in the user's Obelisk right away; for anything big or hard to undo (moving modpacks, deleting a profile), check with the user first.",
            obj(json!({ "command": { "type": "string" }, "args": { "type": "object", "description": "The command's arguments, camelCase, e.g. {\"instanceId\": \"Foundation\"}" },
                "timeout": { "type": "integer", "description": "Seconds to wait for the answer (5-3600, default 120). Past it the command still finishes in the window." } }), &["command"])),
        t("errors", "Errors Obelisk showed the user or hit in the background (newest first, with time, version and where), so they can be fixed at the cause. Stays on this PC.",
            obj(json!({ "limit": { "type": "integer", "description": "1-200, default 30" }, "since": { "type": "integer", "description": "Only errors after this time (ms since 1970)" } }), &[])),
        t("release_submit", "Submit a build of a mod we make to its release line (user rule 00). kind fix = stable: after a 2-cycle gate in each pack's own copy, it goes into EVERY pack that has the mod, one at a time, once the pack is closed, with a safety backup and the old jar kept. kind feature = test build: TEST DUMMY only, until promoted. Builds are rolled back automatically if a crash in that pack names them as the code that was running. Label: fixes are stable, features are test.",
            obj(json!({ "jar": { "type": "string", "description": "The built .jar, in the test bench (C:\\mcw\\...)" },
                "kind": { "type": "string", "enum": ["fix", "feature"] }, "owner": { "type": "string", "description": "Your chat's name" },
                "notes": { "type": "string", "description": "What changed and what to check in game" },
                "mod": { "type": "string", "description": "Mod id, when the jar declares more than one" },
                "version": { "type": "string", "description": "Only when the jar's own version can't be read" } }), &["jar", "kind", "owner"])),
        t("release_status", "Release lines: each mod's latest stable and test build, which version every pack has, the rollout queue (with gate progress), recent installs, failures and rollbacks, and owner notices.", obj(json!({}), &[])),
        t("release_promote", "Promote a test build to stable, which rolls it out to every pack that has the mod. reason user-played: the user played it and it works. reason proven-in-td: checked by Obelisk (3 TEST DUMMY sessions since it went in and no crash naming it).",
            obj(json!({ "mod": { "type": "string" }, "version": { "type": "string" }, "reason": { "type": "string", "enum": ["user-played", "proven-in-td"] } }), &["mod", "version", "reason"])),
        t("release_rollback", "Put back the jar a release replaced in one pack (safety backup first). The pack must be closed.",
            obj(json!({ "mod": { "type": "string" }, "pack": { "type": "string", "description": "The pack's folder name, e.g. Foundation" }, "reason": { "type": "string" } }), &["mod", "pack"])),
        t("release_optout", "Stop (on=true) or resume (on=false) automatic releases into one pack.",
            obj(json!({ "pack": { "type": "string" }, "on": { "type": "boolean" } }), &["pack"])),
        t("copy_pack", "Make a new test copy in the test bench from another copy or \"play\" (skips saves unless saves=true). Test rules are set in the copy: Absolute AFK Cinema off, pauseOnLostFocus false, windowed.",
            obj(json!({ "from": copy, "to": { "type": "string", "description": "New folder name in the test bench" }, "saves": { "type": "boolean" }, "owner": { "type": "string" } }), &["from", "to"])),
    ])
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
                let mut args = msg.pointer("/params/arguments").cloned().unwrap_or(json!({}));
                if name == "wait" && args.get("timeout").is_none() {
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
