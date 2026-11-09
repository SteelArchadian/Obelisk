//! Crash and log diagnosis: reads crash reports, JVM crash logs (hs_err) and the last session's
//! log, and explains in plain words what went wrong, which mod is most likely behind it and what
//! to do about it.

use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReportInfo {
    /// Path relative to the game folder.
    pub file: String,
    /// "crash" (crash-reports/), "jvm" (hs_err_pid*.log) or "log" (logs/latest.log).
    pub kind: String,
    pub time_ms: u64,
}

#[derive(Serialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// "critical", "warning" or "info"
    pub severity: String,
    pub title: String,
    pub detail: String,
    pub fix: String,
    /// What a button next to the finding can do: "memory", "disable", "driver", "java", "shaders", "page", "".
    pub action: String,
    /// Mod file the action applies to, if any.
    pub target: String,
    /// The lines of the file behind this finding (for copying just this issue).
    pub lines: Vec<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Suspect {
    pub mod_id: String,
    /// File in mods/ (may end in .disabled), empty when it couldn't be matched.
    pub file_name: String,
    pub reason: String,
    pub score: u32,
}

#[derive(Serialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Diagnosis {
    pub file: String,
    pub kind: String,
    pub time_ms: u64,
    /// One-line verdict in plain words.
    pub headline: String,
    /// The exception or error line the game printed.
    pub error: String,
    pub findings: Vec<Finding>,
    pub suspects: Vec<Suspect>,
    /// The interesting part of the file (stack trace or matching log lines).
    pub excerpt: Vec<String>,
    /// Memory the game had, in MB, when the file says so.
    pub max_memory_mb: u64,
    /// Logs: where the time went during start-up and world loading.
    pub timeline: Option<crate::insight::Timeline>,
    /// Logs: error lines grouped by the mod they belong to (known-harmless ones are in `noise`).
    pub error_groups: Vec<crate::insight::ErrorGroup>,
    pub noise: Vec<crate::insight::NoiseGroup>,
}

fn modified_ms(p: &Path) -> u64 {
    fs::metadata(p).and_then(|m| m.modified()).ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Crash reports, JVM crash logs and the last session's log, newest first.
pub fn list(game: &Path) -> Vec<ReportInfo> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(game.join("crash-reports")) {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.ends_with(".txt") {
                out.push(ReportInfo { file: format!("crash-reports/{n}"), kind: "crash".into(), time_ms: modified_ms(&e.path()) });
            }
        }
    }
    if let Ok(entries) = fs::read_dir(game) {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("hs_err_pid") && n.ends_with(".log") {
                out.push(ReportInfo { file: n, kind: "jvm".into(), time_ms: modified_ms(&e.path()) });
            }
        }
    }
    let latest = game.join("logs").join("latest.log");
    if latest.is_file() {
        out.push(ReportInfo { file: "logs/latest.log".into(), kind: "log".into(), time_ms: modified_ms(&latest) });
    }
    out.sort_by(|a, b| b.time_ms.cmp(&a.time_ms));
    out
}

fn resolve(game: &Path, rel: &str) -> Result<PathBuf, String> {
    let ok = !rel.contains("..") && !rel.contains('\\') && !rel.contains(':')
        && ((rel.starts_with("crash-reports/") && rel.ends_with(".txt") && rel.matches('/').count() == 1)
            || (rel.starts_with("hs_err_pid") && rel.ends_with(".log") && !rel.contains('/'))
            || rel == "logs/latest.log");
    if !ok {
        return Err("not a crash report".into());
    }
    Ok(game.join(rel))
}

/// The text of one crash report or log (the last 8 MB), for copying.
pub fn read(game: &Path, rel: &str) -> Result<String, String> {
    let bytes = fs::read(resolve(game, rel)?).map_err(|e| e.to_string())?;
    let from = bytes.len().saturating_sub(8 * 1024 * 1024);
    Ok(String::from_utf8_lossy(&bytes[from..]).into_owned())
}

/// Read and explain one file.
pub fn diagnose(game: &Path, rel: &str) -> Result<Diagnosis, String> {
    let path = resolve(game, rel)?;
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    // Logs can be huge; the cause is near the start of a crash report and anywhere in a log.
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(16 * 1024 * 1024)]).into_owned();
    let mods = mod_files(game);
    let kind = if rel.starts_with("hs_err") { "jvm" } else if rel.starts_with("logs/") { "log" } else { "crash" };
    let mut d = analyze(kind, &text, &mods);
    crate::insight::enrich(&mut d, kind, &text, &crate::insight::Index::build(game));
    d.file = rel.to_string();
    d.kind = kind.into();
    d.time_ms = modified_ms(&path);
    Ok(d)
}

/// Files in mods/ by lowercase name (with and without .disabled), for matching stack frames.
fn mod_files(game: &Path) -> HashMap<String, String> {
    let mut m = HashMap::new();
    if let Ok(entries) = fs::read_dir(game.join("mods")) {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            m.insert(n.trim_end_matches(".disabled").to_lowercase(), n.clone());
        }
    }
    m
}

/// Module names in stack frames that are the game, the loader or Java itself, never "the mod".
const NOT_MODS: [&str; 12] = ["minecraft", "neoforge", "fml_loader", "fml", "forge", "java.base", "java.desktop", "mixinextras", "mixin", "bus", "jdk.internal", "datafixerupper"];

const RAN: &str = "The crash happened inside this mod's code.";
const RAN_MIXIN: &str = "The crash happened in code this mod adds to another mod or to the game.";
const ON_STACK: &str = "Involved, but the crash wasn't in its code: the game was only passing through this mod at the time.";
const ON_STACK_MIXIN: &str = "Involved, but the crash wasn't in its code: something this mod adds was only part of the path.";

/// The suspect's own code (or its mixin) was what was running, not just somewhere in the call chain.
pub fn ran(s: &Suspect) -> bool {
    s.reason == RAN || s.reason == RAN_MIXIN
}

/// The analysis itself (no file access), so it can be tested on text.
pub fn analyze(kind: &str, text: &str, mods: &HashMap<String, String>) -> Diagnosis {
    let lines: Vec<&str> = text.lines().collect();
    let mut d = Diagnosis::default();
    let lower = text.to_lowercase();

    // ---- the error line
    let desc = lines.iter().find_map(|l| l.strip_prefix("Description: ")).unwrap_or("").trim().to_string();
    let error_at = lines.iter().position(|l| {
        let t = l.trim_start();
        (t.contains("Exception") || t.contains("Error:") || t.starts_with("java.lang.")) && !t.starts_with("at ") && !t.starts_with("Description")
            && !t.starts_with('#') && !t.contains("[STDERR]")
    });
    if kind == "crash" {
        d.error = error_at.map(|i| lines[i].trim().to_string()).unwrap_or_default();
        let end = error_at.map(|i| (i + 40).min(lines.len())).unwrap_or(0);
        if let Some(i) = error_at {
            d.excerpt = lines[i..end].iter().take_while(|l| !l.starts_with("A detailed walkthrough")).map(|l| l.to_string()).collect();
        }
    }
    d.max_memory_mb = max_memory(text);

    // ---- suspects from stack frames: "at TRANSFORMER/<modid>@<ver>/..." and "~[<jar>%23N!/:?]"
    let mut score: HashMap<String, (u32, String, String)> = HashMap::new();
    // Only the first stack trace (the crash itself); later ones are other threads.
    let frames: Vec<&str> = if kind == "log" { Vec::new() } else {
        let start = error_at.unwrap_or(0);
        lines[start..].iter().skip_while(|l| !l.trim_start().starts_with("at ")).take_while(|l| l.trim_start().starts_with("at ") || l.trim_start().starts_with("..."))
            .filter(|l| l.trim_start().starts_with("at ")).copied().take(60).collect()
    };
    // Blame goes to whose code was actually running: the innermost frame outside Java and its libraries. A mod
    // there, or a mixin handler a mod merged into a game class, threw; mods further down were only passing the
    // call along. When that frame is the game's own code, no mod threw, and none is said to have.
    let mut top_seen = false;
    let mut game_threw = false;
    for (i, f) in frames.iter().enumerate() {
        let w = 30u32.saturating_sub(i as u32 * 2).max(1);
        // Mixin handlers name the mod that injected them: handler$abc000$<modid>$method
        let owners = mixin_owners(f);
        let owner = frame_mod(f);
        let top = !top_seen && (owner.is_some() || !owners.is_empty() || game_frame(f));
        if top {
            top_seen = true;
            game_threw = owner.is_none() && owners.is_empty();
        }
        if let Some((id, jar)) = owner {
            let ran = top && owners.is_empty();
            // A mod's closest frame counts, not how many it has: a big mod deep in the chain mustn't outscore the
            // one whose code threw.
            let e = score.entry(id).or_insert((0, jar, String::new()));
            e.0 = e.0.max(w) + if ran { 60 } else { 0 };
            if ran { e.2 = RAN.into() } else if e.2.is_empty() { e.2 = ON_STACK.into() }
        }
        for id in owners {
            let e = score.entry(id).or_insert((0, String::new(), String::new()));
            e.0 = e.0.max(w) + if top { 60 } else { 0 };
            if top { e.2 = RAN_MIXIN.into() } else if e.2.is_empty() { e.2 = ON_STACK_MIXIN.into() }
        }
    }
    for l in &lines {
        let t = l.trim();
        if let Some(rest) = t.strip_prefix("Suspected Mod:").or_else(|| t.strip_prefix("Suspected Mods:")) {
            for part in rest.split(',') {
                let id = part.split_whitespace().next().unwrap_or("").trim_matches(|c: char| !c.is_alphanumeric() && c != '_').to_lowercase();
                if !id.is_empty() && id != "none" && id != "neoforge" && id != "minecraft" {
                    let e = score.entry(id).or_insert((0, String::new(), String::new()));
                    e.0 += 25;
                    e.2 = "The game itself named this mod as the suspect.".into();
                }
            }
        }
        if let Some(rest) = t.strip_prefix("-- Mod loading issue for: ") {
            let id = rest.trim_end_matches(" --").trim().to_lowercase();
            let e = score.entry(id).or_insert((0, String::new(), String::new()));
            e.0 += 40;
            e.2 = "This mod failed to load.".into();
        }
    }
    let mut connector_ids = Vec::new();
    let mut suspects: Vec<Suspect> = score.into_iter().map(|(mod_id, (score, jar, reason))| {
        if jar.contains("_mapped_moj_") {
            connector_ids.push(mod_id.clone());
        }
        let file_name = match_file(&jar, &mod_id, mods);
        Suspect { mod_id, file_name, reason, score }
    }).collect();
    suspects.sort_by(|a, b| b.score.cmp(&a.score).then(a.mod_id.cmp(&b.mod_id)));
    suspects.truncate(5);
    d.suspects = suspects;
    let top = d.suspects.first().cloned();
    let top_name = top.as_ref().map(|s| s.mod_id.clone()).unwrap_or_default();

    // ---- findings
    let mut f = Vec::new();
    let oom = ["java.lang.outofmemoryerror", "java heap space", "gc overhead limit exceeded", "outofmemoryerror: metaspace", "out of memory"]
        .iter().any(|p| lower.contains(p));
    let native_oom = lower.contains("native memory allocation (malloc) failed") || lower.contains("native memory allocation (mmap) failed")
        || lower.contains("unable to create native thread");
    if native_oom {
        f.push(Finding {
            severity: "critical".into(),
            title: "Your PC ran out of memory".into(),
            detail: "Java asked Windows for memory and didn't get it. This is the PC's RAM and page file, not the game's memory slider.".into(),
            fix: "Close other programs (browsers, Discord), make sure the Windows page file is on, or lower the memory given to Minecraft so the rest of the system has room.".into(),
            action: "memory".into(), ..Default::default()
        });
    } else if oom {
        let have = if d.max_memory_mb > 0 { format!(" It had {} GB.", gb(d.max_memory_mb)) } else { String::new() };
        let want = if d.max_memory_mb > 0 { format!("Try {} GB", gb(d.max_memory_mb + 2048)) } else { "Give it more memory".into() };
        f.push(Finding {
            severity: "critical".into(),
            title: "Minecraft ran out of memory".into(),
            detail: format!("The game used up all the memory it was allowed to use and crashed.{have}"),
            fix: format!("{want} in Profile options → Custom memory. Big modpacks usually need 6–10 GB; don't give it more than about half your PC's RAM."),
            action: "memory".into(), ..Default::default()
        });
    }

    if kind == "jvm" {
        let frame = lines.iter().position(|l| l.starts_with("# Problematic frame:")).and_then(|i| lines.get(i + 1)).map(|l| l.trim_start_matches('#').trim().to_string()).unwrap_or_default();
        let fl = frame.to_lowercase();
        let vendor = if fl.contains("nvoglv") || fl.contains("nvwgf") || fl.contains("nvd3d") { Some("NVIDIA") }
            else if fl.contains("atio") || fl.contains("amdx") || fl.contains("atig") { Some("AMD") }
            else if fl.contains("ig9icd") || fl.contains("igxelp") || fl.contains("igd") || fl.contains("ig75icd") || fl.contains("ig8icd") { Some("Intel") }
            else { None };
        let thread = lines.iter().find(|l| l.starts_with("Current thread")).map(|l| l.to_string()).unwrap_or_default();
        d.error = lines.iter().find(|l| l.contains("EXCEPTION_") || l.contains("SIGSEGV")).map(|l| l.trim_start_matches('#').trim().to_string()).unwrap_or_default();
        d.excerpt = lines.iter().filter(|l| l.starts_with("# ") || l.starts_with("Current thread") || l.starts_with("j  ") || l.starts_with("J ")).take(30).map(|l| l.to_string()).collect();
        // The top Java frame is the call into native code; the first mod frame under it made that call.
        let jframes: Vec<(String, String)> = lines.iter().filter(|l| l.starts_with("j  ") || l.starts_with("J ")).filter_map(|l| jvm_frame(l)).collect();
        let call = jframes.first().map(|(m, _)| m.clone()).unwrap_or_default();
        let (what, func) = native_call(&call);
        let caller = jframes.iter().find(|(_, module)| !game_module(module)).cloned();
        let caller_file = caller.as_ref().map(|(_, id)| match_file("", id, mods)).unwrap_or_default();
        let lib = frame.split_once('[').and_then(|(_, r)| r.split(['+', ']']).next()).unwrap_or("").to_string();
        let addr = lines.iter().find(|l| l.starts_with("siginfo:")).and_then(|l| l.split("address 0x").nth(1))
            .and_then(|h| u64::from_str_radix(h.split_whitespace().next().unwrap_or(""), 16).ok());
        let bad_mem = match addr {
            Some(a) if a < 0x10000 => "tried to read memory that doesn't exist (address 0, a null pointer)".to_string(),
            Some(a) => format!("tried to use memory it had no access to (address 0x{a:x})"),
            None => "crashed".to_string(),
        };
        if let Some(v) = vendor {
            let render = thread.contains("Render thread");
            let from = match &caller { Some((m, id)) => format!(" The call came from {} ({}).", title_case(id), short_method(m)), None => String::new() };
            f.push(Finding {
                severity: "critical".into(),
                title: format!("The {v} graphics driver crashed"),
                detail: format!("Java crashed inside the {v} driver ({frame}){}. Minecraft itself didn't throw an error; the driver did.{from}", if render { " while drawing the game" } else { "" }),
                fix: format!("Update your {v} driver (a clean install helps). If it keeps happening, try without the shaderpack, or turn off mods that change rendering (the suspects below)."),
                action: "driver".into(), target: v.to_string(), ..Default::default()
            });
        } else if let (Some((m, id)), false, false) = (&caller, fl.contains("jvm.dll") || fl.contains("libjvm"), native_oom) {
            let name = title_case(id);
            let place = if lib.is_empty() {
                " The exact spot has no file name: it's code the graphics driver builds while the game runs.".to_string()
            } else { format!(" The exact spot is in {lib}.") };
            let (title, meaning) = if what == "drawing" {
                (format!("{name} made the graphics driver crash"),
                 format!("That means the draw was handed bad data, such as a buffer that was already freed or never filled. Your graphics driver and the other mods are almost certainly fine: this is a bug in {name}."))
            } else {
                (format!("{name} crashed in native code"), format!("That's almost always a bug in {name}, not in Java or your PC."))
            };
            let asked = match what {
                "drawing" => format!("asked the graphics card to draw ({func})"),
                "" => "called into native code".to_string(),
                w => format!("made a {w} call ({func})"),
            };
            f.push(Finding {
                severity: "critical".into(),
                title,
                detail: format!("What happened: {name} ({}) {asked}, and the driver {bad_mem}.{place}\n{meaning}", short_method(m)),
                fix: format!("Update {name}, or disable it to confirm the crash stops.{}", if func.is_empty() { String::new() } else {
                    format!(" If you make this mod: check what {func} is given (buffers freed or not uploaded yet, wrong counts or offsets, or a call from another thread).")
                }),
                action: if caller_file.is_empty() { String::new() } else { "disable".into() }, target: caller_file.clone(), ..Default::default()
            });
        } else if fl.contains("jvm.dll") || fl.contains("libjvm") {
            f.push(Finding {
                severity: "critical".into(),
                title: "Java itself crashed".into(),
                detail: format!("The crash is inside Java ({frame}). That's usually a Java bug or bad RAM/overclock, rarely a mod."),
                fix: "Update Java (Temurin 21 for 1.21), and if you overclock your RAM or CPU, try stock settings.".into(),
                action: "java".into(), ..Default::default()
            });
        } else if !frame.is_empty() && !native_oom {
            f.push(Finding {
                severity: "critical".into(),
                title: "The game crashed outside Java".into(),
                detail: format!("Minecraft runs on Java, but some work is handed to other programs on your PC, like the graphics driver. One of those {bad_mem}, {}.{}",
                    if lib.is_empty() { "in code with no file name (usually code the graphics driver builds while the game runs)".to_string() } else { format!("in {lib}") },
                    if what.is_empty() { String::new() } else { format!(" The game was making a {what} call ({func}) at the time.") }),
                fix: if lib.is_empty() || what == "drawing" { "Update your graphics driver. If it keeps happening, try without the shaderpack, or turn off mods that change rendering.".into() }
                    else { format!("If {lib} came with a mod, update or remove that mod. Otherwise update your graphics driver.") },
                ..Default::default()
            });
        }
        // Java frames name the mod (module id) or at least its package; the mod that made the native call is the suspect,
        // mods further down only started or wrap the game loop.
        if let Some((m, id)) = &caller {
            d.suspects.retain(|s| s.mod_id != *id);
            let did = if what.is_empty() { "called the native code that crashed".to_string() } else { format!("made the {what} call that crashed") };
            d.suspects.insert(0, Suspect { mod_id: id.clone(), file_name: caller_file.clone(), reason: format!("It {did} (in {}).", short_method(m)), score: 10 });
        }
        for (m, module) in &jframes {
            let found = if !module.is_empty() && !game_module(module) { Some((module.clone(), match_file("", module, mods))) }
                else { guess_from_package(&m.to_lowercase(), mods) };
            if let Some((id, file)) = found {
                if !d.suspects.iter().any(|s| s.mod_id == id) {
                    let reason = if caller.is_some() { "Further down the call chain only (it starts or wraps the game loop), so probably not the cause." } else { "This mod's code was running when the crash happened." };
                    d.suspects.push(Suspect { mod_id: id, file_name: file, reason: reason.into(), score: if caller.is_some() { 1 } else { 10 } });
                }
            }
        }
    }

    // Missing or wrong dependencies (NeoForge format).
    let mut missing: Vec<String> = Vec::new();
    for l in &lines {
        if let (Some(a), Some(b)) = (l.find("Mod ID: '"), l.find("Requested by: '")) {
            let id = l[a + 9..].split('\'').next().unwrap_or("");
            let by = l[b + 15..].split('\'').next().unwrap_or("");
            let expected = l.find("Expected range: '").map(|c| l[c + 17..].split('\'').next().unwrap_or("").to_string()).unwrap_or_default();
            let actual = l.find("Actual version: '").map(|c| l[c + 17..].split('\'').next().unwrap_or("").to_string()).unwrap_or_default();
            let what = if actual.contains("MISSING") || actual.is_empty() { format!("{by} needs {id}, which isn't installed") } else { format!("{by} needs {id} {expected}, but {actual} is installed") };
            if !missing.contains(&what) {
                missing.push(what);
            }
        }
    }
    // NeoForge 21: "Failure message: Mod aquamirae requires fragmentum 5.3.0 or above" then "Currently, fragmentum is 5.2.0".
    for (i, l) in lines.iter().enumerate() {
        let Some(rest) = l.split("Failure message: Mod ").nth(1) else { continue };
        let Some((by, need)) = rest.split_once(" requires ") else { continue };
        let id = need.split_whitespace().next().unwrap_or("");
        let range = need[id.len()..].trim();
        let now = lines.get(i + 1).and_then(|n| n.trim().strip_prefix(&format!("Currently, {id} is "))).map(str::trim);
        let what = match now {
            Some(v) if !v.contains("not installed") => format!("{by} needs {id} {range}, but {v} is installed. Update {id}."),
            _ => format!("{by} needs {id} {range}, which isn't installed"),
        };
        if !missing.contains(&what) {
            missing.push(what);
        }
    }
    if !missing.is_empty() {
        f.push(Finding {
            severity: "critical".into(),
            title: if missing.len() == 1 { "A mod is missing something it needs".into() } else { format!("{} mods are missing things they need", missing.len()) },
            detail: missing.join("\n"),
            fix: "Install the missing mods (or the right versions) from Browse, or remove the mod that needs them.".into(),
            ..Default::default()
        });
    }
    if lower.contains("found duplicate mods") || lower.contains("duplicate mods found") || lower.contains("duplicatemodsfoundexception") {
        let dupes: Vec<String> = lines.iter().filter(|l| l.contains("Mod ID: '") && l.contains("from mod files")).map(|l| l.trim().to_string()).collect();
        f.push(Finding {
            severity: "critical".into(),
            title: "The same mod is installed twice".into(),
            detail: if dupes.is_empty() { "Two files provide the same mod.".into() } else { dupes.join("\n") },
            fix: "Keep the newest file and delete the other one (sort Mods by name to spot them).".into(),
            ..Default::default()
        });
    }
    if let Some(l) = lines.iter().find(|l| l.contains("UnsupportedClassVersionError")) {
        let need = l.split("class file version ").nth(1).and_then(|v| v.split('.').next()).and_then(|v| v.parse::<u32>().ok()).map(|v| v - 44);
        f.push(Finding {
            severity: "critical".into(),
            title: "A mod needs a newer Java".into(),
            detail: format!("A mod was built for Java {}, but the game is running an older Java.", need.map(|n| n.to_string()).unwrap_or_else(|| "?".into())),
            fix: "Pick a newer Java for this profile in Prism (Settings → Java), or use the mod version made for your Minecraft version.".into(),
            action: "java".into(), ..Default::default()
        });
    }
    let mixin_fail: Vec<String> = lines.iter().filter(|l| l.contains("Mixin apply") && l.contains("failed") || l.contains("InvalidInjectionException") || l.contains("MixinTransformerError"))
        .take(3).map(|l| l.trim().to_string()).collect();
    if !mixin_fail.is_empty() {
        let owner = lines.iter().find_map(|l| l.split("from mod ").nth(1).map(|s| s.split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap_or("").to_string())).filter(|s| !s.is_empty());
        f.push(Finding {
            severity: "critical".into(),
            title: match &owner { Some(o) => format!("{o} couldn't patch the game (mixin failed)"), None => "A mod couldn't patch the game (mixin failed)".into() },
            detail: "Mods change Minecraft's code with mixins. One of them no longer fits, usually because it's the wrong version for your Minecraft or clashes with another mod.".into(),
            fix: "Update that mod, or check it's the build for your Minecraft and loader version.".into(),
            action: if owner.is_some() { "page".into() } else { String::new() },
            target: owner.and_then(|o| d.suspects.iter().find(|s| s.mod_id == o).map(|s| s.file_name.clone())).unwrap_or_default(),
            lines: mixin_fail.clone(),
        });
    }
    if lower.contains("a single server tick took") || lower.contains("serverhangwatchdog") {
        f.push(Finding {
            severity: "critical".into(),
            title: "The world froze and the game gave up".into(),
            detail: "One server tick ran for over a minute, so Minecraft's watchdog stopped the game on purpose.".into(),
            fix: if top_name.is_empty() { "See the stack below for what was running. Huge farms, chunk loaders or a broken mod are the usual causes.".into() }
                else { format!("{top_name} was running when it froze. Update it or try without it.") },
            ..Default::default()
        });
    }
    if lower.contains("pixel format not accelerated") || lower.contains("wgl: the driver does not appear to support opengl") || lower.contains("glfw error 65542") {
        f.push(Finding {
            severity: "critical".into(),
            title: "OpenGL isn't available".into(),
            detail: "Windows is using a basic display driver, or the game started on the wrong graphics card.".into(),
            fix: "Install your graphics driver, and on laptops set Java (javaw.exe) to the high-performance GPU in Windows Graphics settings.".into(),
            action: "driver".into(), ..Default::default()
        });
    }
    if (lower.contains("iris") || lower.contains("oculus")) && (lower.contains("failed to compile") || lower.contains("shader compilation failed") || lower.contains("program link failed")) {
        f.push(Finding {
            severity: "warning".into(),
            title: "The shaderpack failed to compile".into(),
            detail: "The shaderpack's code doesn't work with your driver or with this Iris version.".into(),
            fix: "Update the shaderpack and Iris, or switch to another shaderpack.".into(),
            action: "shaders".into(), ..Default::default()
        });
    }
    // Fabric mods run through Connector are remapped ("_mapped_moj_" jars).
    if let Some(s) = d.suspects.iter().take(2).find(|s| connector_ids.contains(&s.mod_id)) {
        f.push(Finding {
            severity: "info".into(),
            title: format!("{} is a Fabric mod running through Connector", s.mod_id),
            detail: format!("{} is loaded through Sinytra Connector. Connector works with most Fabric mods, but not all.", s.mod_id),
            fix: "If there's a NeoForge version of that mod, use it instead. Otherwise report it to Connector's issue tracker too.".into(),
            ..Default::default()
        });
    }
    // The crash may be from a build that has since been disabled or replaced.
    if let Some(s) = &top {
        if s.file_name.ends_with(".disabled") {
            f.push(Finding {
                severity: "info".into(),
                title: format!("{} is disabled now", s.mod_id),
                detail: "The mod that crashed here has been turned off since. If the game runs fine now, this crash is already dealt with.".into(),
                fix: String::new(),
                ..Default::default()
            });
        } else if s.file_name.is_empty() {
            f.push(Finding {
                severity: "info".into(),
                title: format!("{} isn't installed any more", s.mod_id),
                detail: "The mod that crashed here is no longer in this profile, so this crash is probably old news.".into(),
                fix: String::new(),
                ..Default::default()
            });
        }
    }

    // ---- last session log: lag, errors
    if kind == "log" {
        let behind: Vec<u64> = lines.iter().filter_map(|l| l.split("Running ").nth(1).filter(|_| l.contains("Can't keep up!")).and_then(|r| r.split("ms").next()).and_then(|n| n.trim().parse().ok())).collect();
        if !behind.is_empty() {
            let worst = behind.iter().max().copied().unwrap_or(0);
            f.push(Finding {
                severity: if worst > 10_000 { "warning".into() } else { "info".into() },
                title: format!("The world lagged {} time{}", behind.len(), if behind.len() == 1 { "" } else { "s" }),
                detail: format!("The server fell behind {} time{}; the worst was {:.1} s behind.", behind.len(), if behind.len() == 1 { "" } else { "s" }, worst as f64 / 1000.0),
                fix: "Fine right after joining a world. If it keeps happening while playing, use a profiler (Lightning or Spark) to see which mod is slow.".into(),
                ..Default::default()
            });
        }
        let errors: Vec<&str> = lines.iter().filter(|l| l.contains("/ERROR]") || l.contains("/FATAL]")).copied().collect();
        let mut by_mod: HashMap<String, usize> = HashMap::new();
        for e in &errors {
            // "[time] [Thread/ERROR] [modid/Logger]: message"
            if let Some(src) = e.split("] [").nth(2).and_then(|s| s.split(['/', ']']).next()).filter(|s| !s.is_empty()) {
                *by_mod.entry(src.to_string()).or_default() += 1;
            }
        }
        if !errors.is_empty() {
            let mut noisy: Vec<(String, usize)> = by_mod.into_iter().collect();
            noisy.sort_by(|a, b| b.1.cmp(&a.1));
            let list: Vec<String> = noisy.iter().take(5).map(|(m, n)| format!("{m}: {n}")).collect();
            f.push(Finding {
                severity: "info".into(),
                title: format!("{} error line{} in the log", errors.len(), if errors.len() == 1 { "" } else { "s" }),
                detail: format!("Most come from: {}", list.join(", ")),
                fix: "Errors in the log don't always mean something is broken; many mods log harmless ones. Look at these if something in game doesn't work.".into(),
                ..Default::default()
            });
            d.excerpt = errors.iter().take(40).map(|l| l.to_string()).collect();
        }
        if d.error.is_empty() {
            d.error = lines.iter().rev().find(|l| l.contains("/FATAL]") || l.contains("Exception in")).map(|l| l.trim().to_string()).unwrap_or_default();
        }
    }

    // A generic "this mod crashed" finding when nothing more specific explains it.
    if kind == "crash" && f.iter().all(|x| x.severity != "critical") && game_threw {
        f.push(Finding {
            severity: "critical".into(),
            title: "The crash happened in Minecraft's own code".into(),
            detail: format!("{} Minecraft itself hit the error, not a mod, so a mod most likely handed it bad data or left something in a bad state.{}",
                plain_error(&d.error), if top.is_some() { " The mods below were in the call chain, the first one most closely; it's a lead, not proof." } else { "" }),
            fix: match &top {
                Some(s) => format!("Disable {} to see whether the crash stops. If it does, update it or report the crash (this file) to its author.", s.mod_id),
                None => "Nothing in the crash points at a mod. Think back to what changed last (a mod added or updated, a config edited) and undo that first.".into(),
            },
            action: if top.is_some() { "disable".into() } else { String::new() },
            target: top.as_ref().map(|s| s.file_name.clone()).unwrap_or_default(),
            ..Default::default()
        });
    }
    if kind == "crash" && f.iter().all(|x| x.severity != "critical") {
        if let Some(s) = &top {
            let name = title_case(&s.mod_id);
            let (detail, fix) = match missing_part(&d.error, &frames) {
                Some((owner, part, class)) if owner == s.mod_id => (
                    format!("{name} tried to use a part of itself ({part}, in {class}) that isn't there. The mod's file is probably damaged or only half updated."),
                    format!("Reinstall {name} (remove it, then add it again from Browse). If that doesn't help, report the crash (this file) to {name}'s author."),
                ),
                Some((owner, part, class)) => {
                    let other = if owner.is_empty() { "another mod".to_string() } else { title_case(&owner) };
                    (format!("{name} tried to use a part of {other} ({part}, in {class}) that the installed {other} doesn't have. \
                        That almost always means {name} was made for a different version of {other}: the two don't fit together."),
                     format!("Update {name} first; its page says which {other} version it needs. If it's already up to date, install the {other} version it asks for, or disable {name} until they match."))
                }
                None => (
                    format!("{}{}", plain_error(&d.error), if s.reason == RAN || s.reason == RAN_MIXIN { " The crash happened in this mod's own code." } else { "" }),
                    format!("Update {name}. If it's already up to date, disable it to confirm, and report the crash (this file) to {name}'s author."),
                ),
            };
            f.push(Finding {
                severity: "critical".into(),
                title: format!("{name} crashed the game"),
                detail, fix,
                action: "disable".into(), target: s.file_name.clone(), ..Default::default()
            });
        }
    }
    d.headline = headline(kind, &f, &desc, top.as_ref());
    d.findings = f;
    d
}

fn gb(mb: u64) -> String {
    let g = mb as f64 / 1024.0;
    if (g - g.round()).abs() < 0.05 { format!("{}", g.round()) } else { format!("{g:.1}") }
}

/// "-Xmx6000m" (JVM Flags line or command line) or the crash report's "Memory: … up to N bytes (M MiB)".
fn max_memory(text: &str) -> u64 {
    for w in text.split_whitespace() {
        if let Some(v) = w.strip_prefix("-Xmx") {
            let (num, unit) = v.split_at(v.find(|c: char| !c.is_ascii_digit()).unwrap_or(v.len()));
            if let Ok(n) = num.parse::<u64>() {
                return match unit.to_lowercase().as_str() { "g" => n * 1024, "k" => n / 1024, "" => n / 1024 / 1024, _ => n };
            }
        }
    }
    text.lines().find(|l| l.trim_start().starts_with("Memory:")).and_then(|l| l.split("up to").nth(1))
        .and_then(|r| r.split('(').nth(1)).and_then(|m| m.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// "at TRANSFORMER/advancedcompassplus@1.0.0/dev.chill…(X.java:156) ~[advancedcompassplus-1.0.0.jar%23503!/:?]"
fn frame_mod(frame: &str) -> Option<(String, String)> {
    let t = frame.trim_start().strip_prefix("at ")?;
    let (layer, rest) = t.split_once('/')?;
    if !matches!(layer, "TRANSFORMER" | "GAME" | "PLUGIN") {
        return None;
    }
    let module = rest.split('/').next()?;
    let id = module.split('@').next()?.to_lowercase();
    if NOT_MODS.iter().any(|n| id == *n || id.starts_with("java.") || id.starts_with("jdk.")) {
        return None;
    }
    let jar = frame.split("~[").nth(1).or_else(|| frame.split(" [").nth(1)).and_then(|j| j.split("%23").next()).unwrap_or("").to_string();
    Some((id, jar))
}

/// A frame of Minecraft itself. The loader counts as a library: a mod calling its API wrongly (reading a config
/// before it's loaded, say) throws inside NeoForge, but the mod that called is what was running.
fn game_frame(frame: &str) -> bool {
    let Some(t) = frame.trim_start().strip_prefix("at ") else { return false };
    match t.split_once('/') {
        Some((layer, rest)) => {
            let module = if matches!(layer, "TRANSFORMER" | "GAME" | "PLUGIN") { rest.split('/').next().unwrap_or("") } else { layer };
            module.split('@').next() == Some("minecraft")
        }
        None => ["net.minecraft.", "com.mojang.blaze3d."].iter().any(|p| t.starts_with(p)),
    }
}

/// Mixin handler methods carry the owning mod's id: handler$abc000$<modid>$name, wrapWithCondition$hpc000$<modid>$…
fn mixin_owners(frame: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in frame.split(['.', '(']) {
        let segs: Vec<&str> = part.split('$').collect();
        if segs.len() >= 4 && segs[1].len() >= 6 && segs[1].chars().all(|c| c.is_ascii_alphanumeric()) && segs[1].ends_with("000") {
            let id = segs[2].to_lowercase();
            if !id.is_empty() && !NOT_MODS.contains(&id.as_str()) {
                out.push(id);
            }
        }
    }
    out
}

/// Installed file for a frame's jar name (Connector's remapped copies end in _mapped_moj_<mc>).
fn match_file(jar: &str, id: &str, mods: &HashMap<String, String>) -> String {
    let j = jar.to_lowercase();
    if let Some(f) = mods.get(&j) {
        return f.clone();
    }
    if let Some(i) = j.find("_mapped_") {
        let base = format!("{}.jar", &j[..i]);
        if let Some(f) = mods.get(&base) {
            return f.clone();
        }
    }
    // Fall back to a file whose name starts with the mod id.
    let idn: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let mut best: Option<&String> = None;
    for (k, v) in mods {
        let kn: String = k.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        if idn.len() >= 3 && kn.starts_with(&idn) && best.is_none_or(|b| v.len() < b.len()) {
            best = Some(v);
        }
    }
    best.cloned().unwrap_or_default()
}

/// hs_err Java frame → (method, module id): "j  dev.boundless.render.VoxelRenderer.submit(...)V+1623 boundless@0.1.0"
/// gives ("dev.boundless.render.VoxelRenderer.submit", "boundless"). Older Javas print no module: "".
fn jvm_frame(l: &str) -> Option<(String, String)> {
    let mut it = l.split_whitespace().skip_while(|w| !w.contains('('));
    let m = it.next()?.split('(').next()?.to_string();
    let module = it.next().filter(|w| w.contains('@')).map(|w| w.split('@').next().unwrap_or("").to_string()).unwrap_or_default();
    Some((m, module))
}

/// Java, the game, the loader and libraries: never the mod to blame. An unknown ("") module isn't a mod either.
fn game_module(m: &str) -> bool {
    m.is_empty() || m.starts_with("java.") || m.starts_with("jdk.") || m.starts_with("org.lwjgl") || NOT_MODS.contains(&m)
        || matches!(m, "fabricloader" | "org.spongepowered.mixin")
}

/// What kind of native call a frame is, in plain words, and the function: ("drawing", "glDrawElements").
fn native_call(m: &str) -> (&'static str, String) {
    let func = m.rsplit('.').next().unwrap_or("");
    // LWJGL's "nglFoo" is the raw form of "glFoo".
    let func = func.strip_prefix('n').filter(|f| f.starts_with("gl") || f.starts_with("vk") || f.starts_with("al")).unwrap_or(func).to_string();
    let what = if m.starts_with("org.lwjgl.opengl") || m.starts_with("org.lwjgl.vulkan") { "drawing" }
        else if m.starts_with("org.lwjgl.glfw") { "window or input" }
        else if m.starts_with("org.lwjgl.openal") { "sound" }
        else { "" };
    (what, if what.is_empty() { String::new() } else { func })
}

/// A NoSuchField/NoSuchMethodError, as (mod that owns the class, missing part, class short name). The owner comes from
/// the frames that run that class ("" when none do), "minecraft" for the game's classes.
/// "Class a.b.ShaderLoader does not have member field 'boolean noMaterial'" → ("sodium", "noMaterial", "ShaderLoader").
fn missing_part(error: &str, frames: &[&str]) -> Option<(String, String, String)> {
    let (kind, msg) = error.split_once(':')?;
    let msg = msg.trim();
    let (class, part) = if kind.ends_with("NoSuchFieldError") && msg.starts_with("Class ") {
        let class = msg.strip_prefix("Class ")?.split_whitespace().next()?;
        (class.to_string(), msg.rsplit(' ').next()?.trim_matches('\'').to_string())
    } else if kind.ends_with("NoSuchFieldError") || kind.ends_with("NoSuchMethodError") {
        // "'void a.b.Foo.bar(int)'" or "'boolean a.b.Foo.baz'"
        let sig = msg.trim_matches('\'').split_whitespace().last()?.split('(').next()?;
        let (class, part) = sig.rsplit_once('.')?;
        (class.to_string(), part.to_string())
    } else {
        return None;
    };
    let class = class.replace('/', ".");
    let owner = if class.starts_with("net.minecraft.") || class.starts_with("com.mojang.") { "minecraft".to_string() } else {
        frames.iter().find(|f| f.contains(&format!("/{class}."))).and_then(|f| frame_mod(f)).map(|(id, _)| id).unwrap_or_default()
    };
    let short = class.rsplit('.').next().unwrap_or(&class).to_string();
    Some((owner, part, short))
}

/// "dev.boundless.render.VoxelRenderer.submit" → "VoxelRenderer.submit".
fn short_method(m: &str) -> String {
    let p: Vec<&str> = m.rsplitn(3, '.').collect();
    if p.len() >= 2 { format!("{}.{}", p[1], p[0]) } else { m.to_string() }
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// hs_err Java frame "me.cortex.voxy.client.Foo.bar" → an installed file whose name contains a package word.
fn guess_from_package(pkg: &str, mods: &HashMap<String, String>) -> Option<(String, String)> {
    let parts: Vec<&str> = pkg.split('.').collect();
    if parts.len() < 3 || matches!(parts[0], "java" | "jdk" | "sun") || pkg.starts_with("net.minecraft") || pkg.starts_with("com.mojang") || pkg.starts_with("net.neoforged") || pkg.starts_with("org.lwjgl") {
        return None;
    }
    for word in parts.iter().skip(1).take(3) {
        if word.len() < 4 {
            continue;
        }
        if let Some((_, v)) = mods.iter().find(|(k, _)| k.contains(word)) {
            return Some((word.to_string(), v.clone()));
        }
    }
    None
}

fn plain_error(e: &str) -> String {
    let e = e.trim();
    if e.is_empty() {
        return String::new();
    }
    let (kind, msg) = e.split_once(':').unwrap_or((e, ""));
    let short = kind.rsplit('.').next().unwrap_or(kind);
    let what = match short {
        "NullPointerException" => "It tried to use something that wasn't there (a null value).",
        "ConcurrentModificationException" => "Two parts of the game changed the same list at the same time.",
        "ClassCastException" => "It mixed up two kinds of objects, usually a clash between two mods.",
        "ArrayIndexOutOfBoundsException" | "IndexOutOfBoundsException" => "It read past the end of a list.",
        "IllegalStateException" => "It found the game in a state it didn't expect.",
        "NoSuchMethodError" | "NoSuchFieldError" | "NoClassDefFoundError" | "ClassNotFoundException" => "It expected code that isn't there: usually a missing library or a version mismatch.",
        "StackOverflowError" => "It got stuck calling itself in a loop.",
        _ => "",
    };
    let msg = msg.trim();
    if what.is_empty() { format!("Error: {short}{}", if msg.is_empty() { String::new() } else { format!(" ({msg})") }) } else { what.to_string() }
}

fn headline(kind: &str, f: &[Finding], desc: &str, top: Option<&Suspect>) -> String {
    if let Some(c) = f.iter().find(|x| x.severity == "critical") {
        return c.title.clone();
    }
    match kind {
        "log" => if f.is_empty() { "The last session looks clean".into() } else { "The last session ran, with some warnings".into() },
        _ => match top {
            Some(s) => format!("Crash involving {}", s.mod_id),
            None if !desc.is_empty() => format!("Crash: {desc}"),
            None => "Crash with no clear cause".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods() -> HashMap<String, String> {
        [("advancedcompassplus-1.0.0.jar", "advancedcompassplus-1.0.0.jar"),
         ("advancedcompass.fabric.1.21.1-1.2.18.jar", "advancedcompass.fabric.1.21.1-1.2.18.jar"),
         ("gnetum-3.3.6.jar", "gnetum-3.3.6.jar")]
            .into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn names_the_mod_from_frames_and_mixins() {
        let t = "Description: Unexpected error\n\njava.lang.NullPointerException: Cannot invoke \"Font.width(String)\" because \"font\" is null\n\
\tat TRANSFORMER/advancedcompassplus@1.0.0/dev.chill.acplus.render.ThemeRenderer.drawCardinal(ThemeRenderer.java:156) ~[advancedcompassplus-1.0.0.jar%23503!/:?] {re:mixin}\n\
\tat TRANSFORMER/advancedcompass@1.2.18/dlovin.advancedcompass.gui.renderers.CompassRenderer.handler$jan000$advancedcompassplus$acplus$cardinal(CompassRenderer.java:561) ~[advancedcompass.fabric.1.21.1-1.2.18_mapped_moj_1.21.1.jar%231450!/:?] {re:mixin}\n\
\tat TRANSFORMER/gnetum@3.3.6/me.decce.gnetum.GuiHelper.renderLayers(GuiHelper.java:68) ~[gnetum-3.3.6.jar%23630!/:?] {re:mixin}\n\
\tat TRANSFORMER/minecraft@1.21.1/net.minecraft.client.gui.Gui.render(Gui.java:1) ~[client.jar%23489!/:?] {}\n";
        let d = analyze("crash", t, &mods());
        assert_eq!(d.suspects[0].mod_id, "advancedcompassplus");
        assert_eq!(d.suspects[0].file_name, "advancedcompassplus-1.0.0.jar");
        let ac = d.suspects.iter().find(|s| s.mod_id == "advancedcompass").unwrap();
        assert_eq!(ac.file_name, "advancedcompass.fabric.1.21.1-1.2.18.jar");
        assert!(d.headline.to_lowercase().contains("advancedcompassplus"), "{}", d.headline);
        assert!(d.suspects[0].score > d.suspects[1].score + 20);
        assert!(d.findings.iter().any(|f| f.action == "disable" && f.target == "advancedcompassplus-1.0.0.jar"));
        assert!(d.error.starts_with("java.lang.NullPointerException"));
    }

    #[test]
    fn a_crash_in_game_code_blames_no_mod_outright() {
        let t = "Description: Rendering entity\n\njava.lang.NullPointerException: oops\n\
\tat java.base/java.util.Objects.requireNonNull(Unknown Source)\n\
\tat TRANSFORMER/minecraft@1.21.1/net.minecraft.client.renderer.entity.EntityRenderer.render(EntityRenderer.java:9) ~[client.jar%23489!/:?] {}\n\
\tat TRANSFORMER/gnetum@3.3.6/me.decce.gnetum.GuiHelper.renderLayers(GuiHelper.java:68) ~[gnetum-3.3.6.jar%23630!/:?] {re:mixin}\n";
        let d = analyze("crash", t, &mods());
        assert_eq!(d.suspects[0].mod_id, "gnetum");
        assert_eq!(d.suspects[0].reason, ON_STACK);
        assert_eq!(d.findings[0].title, "The crash happened in Minecraft's own code");
        assert_eq!(d.findings[0].target, "gnetum-3.3.6.jar");
    }

    #[test]
    fn a_mixin_handler_in_game_code_is_what_ran() {
        let t = "Description: Ticking\n\njava.lang.IllegalStateException: bad\n\
\tat TRANSFORMER/minecraft@1.21.1/net.minecraft.world.level.Level.handler$abc000$gnetum$onTick(Level.java:5) ~[client.jar%23489!/:?] {re:mixin}\n\
\tat TRANSFORMER/advancedcompassplus@1.0.0/dev.chill.acplus.Foo.bar(Foo.java:1) ~[advancedcompassplus-1.0.0.jar%23503!/:?] {}\n";
        let d = analyze("crash", t, &mods());
        assert_eq!(d.suspects[0].mod_id, "gnetum");
        assert_eq!(d.suspects[0].reason, RAN_MIXIN);
        assert_eq!(d.suspects[1].reason, ON_STACK);
        assert!(d.findings.iter().all(|f| !f.title.contains("Minecraft's own code")));
    }

    #[test]
    fn out_of_memory_says_add_memory() {
        let t = "Description: Unexpected error\njava.lang.OutOfMemoryError: Java heap space\n\tat java.base/java.util.Arrays.copyOf(Unknown Source)\nJVM Flags: 3 total; -Xss1M -Xmx6000m -Xms256m\n";
        let d = analyze("crash", t, &HashMap::new());
        assert_eq!(d.findings[0].title, "Minecraft ran out of memory");
        assert_eq!(d.max_memory_mb, 6000);
        assert!(d.findings[0].fix.contains("7.9") || d.findings[0].fix.contains("8"), "{}", d.findings[0].fix);
        assert_eq!(d.headline, "Minecraft ran out of memory");
    }

    #[test]
    fn jvm_crash_in_nvidia_driver() {
        let t = "#  EXCEPTION_ACCESS_VIOLATION (0xc0000005) at pc=0x1, pid=1, tid=2\n# Problematic frame:\n# C  [nvoglv64.dll+0xb80d80]\nCurrent thread (0x1):  JavaThread \"Render thread\"\n";
        let d = analyze("jvm", t, &HashMap::new());
        assert_eq!(d.findings[0].title, "The NVIDIA graphics driver crashed");
        assert_eq!(d.findings[0].action, "driver");
    }

    #[test]
    fn neoforge_too_old_dependency() {
        let t = "Description: Mod loading failures have occurred\n\nnet.neoforged.neoforge.logging.CrashReportExtender$ModLoadingCrashException: Mod loading has failed\n\n-- Mod loading issue for: aquamirae --\nDetails:\n\tFailure message: Mod aquamirae requires fragmentum 5.3.0 or above\n\t\tCurrently, fragmentum is 5.2.0\n";
        let d = analyze("crash", t, &HashMap::new());
        let f = &d.findings[0];
        assert_eq!(f.title, "A mod is missing something it needs");
        assert_eq!(f.detail, "aquamirae needs fragmentum 5.3.0 or above, but 5.2.0 is installed. Update fragmentum.");
        assert!(!d.findings.iter().any(|x| x.title.contains("crashed the game")));
    }

    #[test]
    fn missing_dependency_and_lag() {
        let t = "\tMod ID: 'balm', Requested by: 'waystones', Expected range: '[21.0,)', Actual version: '[MISSING]'\n";
        let d = analyze("crash", t, &HashMap::new());
        assert!(d.findings[0].detail.contains("waystones needs balm"));
        let log = "[12:00:00] [Server thread/WARN] [minecraft/MinecraftServer]: Can't keep up! Is the server overloaded? Running 2500ms or 50 ticks behind\n[12:00:01] [Render thread/ERROR] [sodium/]: oops\n";
        let d = analyze("log", log, &HashMap::new());
        assert!(d.findings.iter().any(|f| f.title.starts_with("The world lagged 1 time")));
        assert!(d.findings.iter().any(|f| f.title.starts_with("1 error line") && f.detail.contains("sodium: 1")));
        assert!(d.suspects.is_empty());
    }

    #[test]
    fn only_crash_files_can_be_read() {
        let g = Path::new("C:/x");
        assert!(resolve(g, "crash-reports/crash-1.txt").is_ok());
        assert!(resolve(g, "logs/latest.log").is_ok());
        assert!(resolve(g, "hs_err_pid1.log").is_ok());
        assert!(resolve(g, "crash-reports/../options.txt").is_err());
        assert!(resolve(g, "config/a.txt").is_err());
    }

    fn files(names: &[&str]) -> HashMap<String, String> {
        names.iter().map(|n| (n.to_string(), n.to_string())).collect()
    }

    #[test]
    fn a_native_crash_names_the_mod_that_made_the_draw_call() {
        let t = "#  EXCEPTION_ACCESS_VIOLATION (0xc0000005) at pc=0x000001c94a85fb5a, pid=57996, tid=54740\n\
# Problematic frame:\n# C  0x000001c94a85fb5a\n#\n\
Current thread (0x000001c89e49b040):  JavaThread \"Render thread\"        [_thread_in_native, id=54740]\n\
J 78332  org.lwjgl.opengl.GL32C.nglMultiDrawElementsBaseVertex(IJIJIJ)V org.lwjgl.opengl@3.3.3+5 (0 bytes) @ 0x1\n\
j  dev.boundless.render.VoxelRenderer.submit(Ldev/boundless/render/VoxelRenderer$DrawList;IIZDDD)V+1623 boundless@0.1.0-prototype\n\
J 86338 c1 net.minecraft.client.Minecraft.runTick(Z)V minecraft@1.21.1 (1277 bytes) @ 0x2\n\
j  me.decce.ixeris.RenderThreadStarter+0x0000000008a04000.run()V+13 ixeris@4.6.11+1.21.1-neoforge\n\
siginfo: EXCEPTION_ACCESS_VIOLATION (0xc0000005), reading address 0x0000000000000000\n";
        let d = analyze("jvm", t, &files(&["boundless-0.1.0-prototype.jar", "ixeris-4.6.11.jar"]));
        assert_eq!(d.findings[0].title, "Boundless made the graphics driver crash");
        assert!(d.findings[0].detail.contains("glMultiDrawElementsBaseVertex") && d.findings[0].detail.contains("null pointer"), "{}", d.findings[0].detail);
        assert_eq!(d.findings[0].target, "boundless-0.1.0-prototype.jar");
        assert_eq!(d.suspects[0].mod_id, "boundless");
        assert_eq!(d.suspects[1].mod_id, "ixeris");
        assert!(d.suspects[1].reason.contains("probably not the cause"));
    }

    #[test]
    fn a_missing_field_blames_the_mod_that_wanted_it() {
        let t = "Description: Unexpected error\n\njava.lang.NoSuchFieldError: Class net.caffeinemc.mods.sodium.client.gl.shader.ShaderLoader does not have member field 'boolean noMaterial'\n\
\tat TRANSFORMER/sodium@0.8.13+mc1.21.1/net.caffeinemc.mods.sodium.client.gl.shader.ShaderLoader.handler$gno000$windowportals$noMaterial(ShaderLoader.java:1020) ~[sodium.jar%23497!/:?] {re:mixin}\n\
\tat TRANSFORMER/sodium@0.8.13+mc1.21.1/net.caffeinemc.mods.sodium.client.gl.shader.ShaderLoader.getShaderSource(ShaderLoader.java:48) ~[sodium.jar%23497!/:?] {re:mixin}\n\
\tat TRANSFORMER/sodium@0.8.13+mc1.21.1/net.caffeinemc.mods.sodium.client.gl.shader.ShaderLoader.loadShader(ShaderLoader.java:30) ~[sodium.jar%23497!/:?] {re:mixin}\n\
\tat TRANSFORMER/sodium@0.8.13+mc1.21.1/net.caffeinemc.mods.sodium.client.render.chunk.ShaderChunkRenderer.createShader(ShaderChunkRenderer.java:44) ~[sodium.jar%23497!/:?] {re:mixin}\n\
\tat TRANSFORMER/sodium@0.8.13+mc1.21.1/net.caffeinemc.mods.sodium.client.render.chunk.ShaderChunkRenderer.begin(ShaderChunkRenderer.java:85) ~[sodium.jar%23497!/:?] {re:mixin}\n\
\tat TRANSFORMER/minecraft@1.21.1/net.minecraft.client.renderer.LevelRenderer.renderLevel(LevelRenderer.java:965) ~[client.jar%23495!/:?] {re:mixin}\n\
\tat TRANSFORMER/windowportals@0.1.0-test/dev.windowportals.client.PortalRenderer.pass(PortalRenderer.java:333) ~[windowportals-0.1.0-test.jar%23792!/:?] {re:mixin}\n";
        let d = analyze("crash", t, &files(&["sodium.jar", "windowportals-0.1.0-test.jar"]));
        assert_eq!(d.suspects[0].mod_id, "windowportals");
        assert_eq!(d.findings[0].title, "Windowportals crashed the game");
        assert!(d.findings[0].detail.contains("a part of Sodium (noMaterial, in ShaderLoader)"), "{}", d.findings[0].detail);
    }
}
