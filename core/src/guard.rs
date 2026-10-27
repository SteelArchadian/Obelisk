//! Checks on files that come from someone else's PC (share codes, collaborative packs, pack files). Nothing is
//! trusted because the sender says so: the receiving Obelisk decides what a file is and looks inside it.
//!
//! Three layers:
//! - `blocked`: Windows programs, scripts and shortcuts are never installed. No Minecraft pack needs them.
//! - `inspect`: jars and archives are opened (nested jars too) and their classes searched for what malware
//!   that targeted Minecraft mods did: known addresses of the 2023 "fractureiser" campaign, reading browser or
//!   launcher logins, posting to Discord webhooks, hidden Windows programs. Serious findings refuse the file.
//! - `defender_scan`: Windows Defender scans what arrived, before the pack is used.

use std::io::Read;
use std::path::Path;

/// Never installed from another PC: things Windows runs directly.
const BLOCKED_EXT: &[&str] = &[
    ".exe", ".dll", ".com", ".scr", ".pif", ".bat", ".cmd", ".ps1", ".psm1", ".psd1", ".vbs", ".vbe", ".wsf", ".wsh",
    ".hta", ".msi", ".msp", ".msc", ".lnk", ".url", ".reg", ".cpl", ".jse", ".sys", ".drv", ".ocx", ".application",
    ".appref-ms", ".gadget", ".scf", ".inf",
];

/// Code that runs in the game: jars and classes, scripts for script mods, anything in the code folders.
const CODE_EXT: &[&str] = &[".jar", ".class", ".js", ".zs", ".lua", ".py", ".groovy", ".kts", ".sh"];
const CODE_DIRS: &[&str] = &["mods", "kubejs", "scripts"];
/// Plain data that may sit among the code (a mod's notes, Prism's mods/.index/*.pw.toml records, kubejs data).
const DATA_EXT: &[&str] = &[".toml", ".json", ".json5", ".txt", ".md", ".cfg", ".properties", ".png", ".ogg", ".nbt", ".snbt", ".mcmeta"];

fn lower(path: &str) -> String {
    path.to_ascii_lowercase()
}

/// A file type that is never accepted from another PC, or a name Windows would quietly turn into something else:
/// it drops trailing dots and spaces ("setup.exe." lands as setup.exe), treats CON, NUL, COM1… as devices, and
/// refuses <>"|?* and control characters.
pub fn blocked(path: &str) -> bool {
    let p = lower(path);
    BLOCKED_EXT.iter().any(|e| p.ends_with(e)) || p.split('/').any(bad_segment)
}

fn bad_segment(seg: &str) -> bool {
    let stem = seg.split('.').next().unwrap_or("");
    let device = matches!(stem, "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4 && (stem.starts_with("com") || stem.starts_with("lpt")) && stem.as_bytes()[3].is_ascii_digit());
    seg.is_empty() || seg.ends_with('.') || seg.ends_with(' ') || device
        || seg.chars().any(|c| c.is_control() || "<>\"|?*".contains(c))
}

/// Code that would run in the game (needs a site's word for it, or the user's choice, plus inspection).
pub fn risky(path: &str) -> bool {
    let p = lower(path);
    let first = p.split('/').next().unwrap_or("");
    CODE_EXT.iter().any(|e| p.ends_with(e)) || (CODE_DIRS.contains(&first) && !DATA_EXT.iter().any(|e| p.ends_with(e)))
}

/// Prism's record of where a mod came from (mods/.index/<mod>.pw.toml). Prism downloads from its url when the
/// mod is updated, so one from another PC is kept only when every url in it is Modrinth's or CurseForge's.
pub fn mod_record(path: &str) -> bool {
    let p = lower(path);
    p.starts_with("mods/.index/") && p.ends_with(".pw.toml")
}

pub fn mod_record_ok(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else { return false };
    text.lines().filter_map(|l| l.trim().strip_prefix("url")).filter_map(|r| r.trim_start().strip_prefix('='))
        .all(|v| crate::packcode::site_url_ok(v.trim().trim_matches(|c| c == '"' || c == '\'')))
}

/// A config file that seems to hold a secret of the sharer's: a bot token, a password, an API key, a webhook.
/// Those stay on the sharer's PC when a pack is shared (by code or zip), whatever else the pack holds.
pub fn holds_secret(path: &str, bytes: &[u8]) -> bool {
    let p = lower(path);
    let first = p.split('/').next().unwrap_or("");
    let text_ext = [".toml", ".json", ".json5", ".cfg", ".conf", ".properties", ".yml", ".yaml", ".txt", ".ini", ".snbt"];
    if !matches!(first, "config" | "defaultconfigs" | "kubejs" | "scripts") || !text_ext.iter().any(|e| p.ends_with(e)) {
        return false;
    }
    let Ok(text) = std::str::from_utf8(bytes) else { return false };
    text.lines().any(|line| {
        let l = line.to_ascii_lowercase();
        if l.contains("discord.com/api/webhooks/") || l.contains("discordapp.com/api/webhooks/") {
            return true;
        }
        let keyed = ["token", "secret", "password", "passwd", "api_key", "apikey", "api-key", "webhook", "client_secret"]
            .iter().any(|k| l.contains(k));
        let Some((_, value)) = line.split_once(['=', ':']) else { return false };
        let v = value.trim().trim_end_matches(',').trim().trim_matches(['"', '\'', '`']);
        keyed && v.len() >= 20 && v.chars().all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
            && v.chars().collect::<std::collections::HashSet<_>>().len() > 6
    })
}

/// What inspecting a file found, in plain words.
#[derive(serde::Serialize, Debug, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Findings {
    /// Reasons to refuse the file outright.
    pub serious: Vec<String>,
    /// Things legitimate mods also do, shown so the user can judge.
    pub notes: Vec<String>,
}

impl Findings {
    fn add(&mut self, serious: bool, what: String) {
        let list = if serious { &mut self.serious } else { &mut self.notes };
        if !list.contains(&what) {
            list.push(what);
        }
    }
}

/// Byte patterns in class files. (pattern, serious, what it means)
const SIGNS: &[(&str, bool, &str)] = &[
    // fractureiser (2023): stage-0 loader addresses and names.
    ("85.217.144.130", true, "contains an address used by the fractureiser malware"),
    ("107.189.3.101", true, "contains an address used by the fractureiser malware"),
    ("files-8ie.pages.dev", true, "contains an address used by the fractureiser malware"),
    ("skyrage", true, "mentions Skyrage, a known Minecraft malware family"),
    // Stealing logins.
    ("Login Data", true, "reads browsers' saved passwords"),
    ("os_crypt", true, "reads the key browsers encrypt saved passwords with"),
    ("Local Storage/leveldb", true, "reads Discord's or a browser's local storage (where login tokens live)"),
    ("Local Storage\\leveldb", true, "reads Discord's or a browser's local storage (where login tokens live)"),
    ("launcher_accounts", true, "reads the Minecraft launcher's saved accounts"),
    ("microsoft_accounts.json", true, "reads a launcher's saved Microsoft accounts"),
    ("discord.com/api/webhooks", true, "posts to a Discord webhook (how stolen data is usually sent)"),
    ("discordapp.com/api/webhooks", true, "posts to a Discord webhook (how stolen data is usually sent)"),
    // Things real mods do too.
    ("java/lang/ProcessBuilder", false, "can start other programs"),
    ("java/net/URLClassLoader", false, "can load code from outside its own jar"),
    ("defineClass", false, "can create code while running"),
];

/// Look inside a file from another PC. Jars and zips are opened, nested ones too (to a depth of 3).
pub fn inspect(path: &str, bytes: &[u8]) -> Findings {
    let mut f = Findings::default();
    inspect_into(path, bytes, 0, &mut f);
    f
}

fn inspect_into(path: &str, bytes: &[u8], depth: u32, f: &mut Findings) {
    let p = lower(path);
    if bytes.starts_with(b"MZ") {
        f.add(true, format!("{path} is a Windows program"));
        return;
    }
    if p.ends_with(".class") {
        scan_bytes(bytes, f);
        return;
    }
    if !(p.ends_with(".jar") || p.ends_with(".zip") || bytes.starts_with(b"PK\x03\x04")) {
        if CODE_EXT.iter().any(|e| p.ends_with(e)) {
            scan_bytes(bytes, f);
        }
        return;
    }
    let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
        // Named like an archive but isn't one: the game can't load it as a mod, so it has no business there.
        f.add(false, format!("{path} isn't a valid jar or zip"));
        scan_bytes(bytes, f);
        return;
    };
    for i in 0..zip.len().min(200_000) {
        let Ok(mut entry) = zip.by_index(i) else { continue };
        // Folders inside a jar ("META-INF/") aren't files: nothing to look at.
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let n = lower(&name);
        if n.ends_with(".dll") || n.ends_with(".so") || n.ends_with(".dylib") {
            f.add(false, "includes native code (some mods ship this for real features)".into());
            continue;
        }
        if blocked(&name) {
            f.add(true, format!("hides a Windows program or script inside: {name}"));
            continue;
        }
        let nested = n.ends_with(".jar") || n.ends_with(".zip");
        if !(n.ends_with(".class") || nested) || entry.size() > 64 * 1024 * 1024 {
            continue;
        }
        let mut b = Vec::with_capacity(entry.size() as usize);
        if (&mut entry).take(64 * 1024 * 1024).read_to_end(&mut b).is_err() {
            continue;
        }
        if b.starts_with(b"MZ") {
            f.add(true, format!("hides a Windows program inside: {name}"));
        } else if nested && depth < 3 {
            inspect_into(&name, &b, depth + 1, f);
        } else if n.ends_with(".class") {
            scan_bytes(&b, f);
        }
    }
}

fn scan_bytes(b: &[u8], f: &mut Findings) {
    for (pat, serious, what) in SIGNS {
        if contains(b, pat.as_bytes()) {
            f.add(*serious, (*what).to_string());
        }
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

/// What Windows Defender said about a file or folder.
#[derive(serde::Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase", tag = "result", content = "detail")]
pub enum Scan {
    Clean,
    Threat(String),
    /// Defender isn't there or couldn't scan (another antivirus may be in charge).
    Unavailable(String),
}

/// Scan a file or folder with Windows Defender. Defender deals with what it finds in its usual way (quarantine), and
/// Obelisk then refuses the pack: an exit code of 2 means a threat was found, whether or not it was cleaned up.
pub fn defender_scan(path: &Path) -> Scan {
    defender_scan_watched(path, |_| {}, || false)
}

/// `defender_scan`, telling `tick` the seconds spent once a second, and ending the scan if `stop` says so
/// (the caller then throws away what was being scanned; a stopped scan never counts as clean).
pub fn defender_scan_watched(path: &Path, tick: impl Fn(u64), stop: impl Fn() -> bool) -> Scan {
    let exe = Path::new(&std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into()))
        .join("Windows Defender").join("MpCmdRun.exe");
    if !exe.is_file() {
        return Scan::Unavailable("Windows Defender isn't installed here".into());
    }
    let mut c = std::process::Command::new(exe);
    c.args(["-Scan", "-ScanType", "3", "-File"]).arg(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    let mut child = match c.spawn() {
        Ok(ch) => ch,
        Err(e) => return Scan::Unavailable(format!("couldn't start Defender: {e}")),
    };
    // Its report is read alongside, so a long one can't fill the pipe and stall the scan.
    let mut out = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = out.as_mut() { let _ = std::io::Read::read_to_string(o, &mut s); }
        s
    });
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Ok(st),
            Ok(None) if stop() => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("stopped".to_string());
            }
            Ok(None) => {
                std::thread::sleep(std::time::Duration::from_millis(500));
                tick(start.elapsed().as_secs());
            }
            Err(e) => break Err(e.to_string()),
        }
    };
    let text = reader.join().unwrap_or_default();
    match status {
        Ok(st) => {
            match st.code() {
                Some(0) => Scan::Clean,
                Some(2) => Scan::Threat(text.lines().filter(|l| l.contains("Threat") || l.contains("file:")).take(6)
                    .map(str::trim).collect::<Vec<_>>().join("; ")),
                code => Scan::Unavailable(format!("Defender couldn't scan (exit {code:?})")),
            }
        }
        Err(e) => Scan::Unavailable(format!("Defender didn't finish: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn jar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (n, b) in entries {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    #[test]
    fn kinds() {
        assert!(blocked("mods/setup.EXE") && blocked("config/x.ps1") && blocked("mods/a.lnk"));
        assert!(!blocked("mods/sodium.jar") && !blocked("kubejs/server_scripts/a.js"));
        assert!(risky("mods/a.jar") && risky("kubejs/startup_scripts/x.js") && risky("config/loader.jar"));
        assert!(!risky("config/sodium-options.json") && !risky("resourcepacks/faithful.zip"));
        // Names Windows would quietly change into something else.
        assert!(blocked("mods/setup.exe.") && blocked("mods/setup.exe ") && blocked("config/CON.txt") && blocked("mods/a?.jar"));
        assert!(blocked("mods//x.jar") && !blocked("config/console.toml") && !blocked("mods/com.example.jar"));
        // Prism's records of where each mod came from are data, but a disabled jar is still code.
        assert!(!risky("mods/.index/sodium.pw.toml") && risky("mods/a.jar.disabled"));
        assert!(mod_record("mods/.index/sodium.pw.toml"));
        assert!(mod_record_ok(b"[download]\nurl = \"https://cdn.modrinth.com/data/AANobbMI/versions/x/sodium.jar\"\nhash-format = \"sha1\""));
        assert!(mod_record_ok(b"name = \"Sodium\"\n[update.modrinth]\nmod-id = \"AANobbMI\""));
        assert!(!mod_record_ok(b"[download]\nurl = \"https://evil.example/sodium.jar\""));
    }

    #[test]
    fn folders_inside_a_jar_are_fine() {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut z = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default();
        z.add_directory("META-INF/", o).unwrap();
        z.add_directory("dev/chill/acplus/", o).unwrap();
        z.start_file("dev/chill/acplus/Mod.class", o).unwrap();
        std::io::Write::write_all(&mut z, b"\xca\xfe\xba\xbe plain").unwrap();
        z.finish().unwrap();
        assert!(inspect("mods/advancedcompassplus.jar", buf.get_ref()).serious.is_empty());
    }

    #[test]
    fn secrets_stay_home() {
        assert!(holds_secret("config/discordintegration.toml", b"botToken = \"MTA4NzE2NTY3ODkwMTIzNDU2Nw.Gx1yZ2.abcdefghijklmnop\""));
        assert!(holds_secret("config/chat.json", b"{\"url\": \"https://discord.com/api/webhooks/1/abc\"}"));
        assert!(!holds_secret("config/sodium.toml", b"tokenBucketSize = 20\npassword_hint = \"\""));
        assert!(!holds_secret("mods/x.jar", b"token = abcdefghijklmnopqrstuvwxyz"));
    }

    #[test]
    fn finds_malware_signs_even_nested() {
        let evil_class = b"\xca\xfe\xba\xbe....java/net/URLClassLoader...85.217.144.130....";
        let inner = jar(&[("dev/Utility.class", evil_class)]);
        let outer = jar(&[("META-INF/jarjar/lib.jar", &inner), ("a/Mod.class", b"\xca\xfe\xba\xbe plain")]);
        let f = inspect("mods/cool.jar", &outer);
        assert!(f.serious.iter().any(|s| s.contains("fractureiser")), "{f:?}");
        assert!(f.notes.iter().any(|s| s.contains("load code")), "{f:?}");
    }

    #[test]
    fn finds_hidden_programs_and_credential_theft() {
        let f = inspect("mods/x.jar", &jar(&[("payload.bin", b"MZ\x90\x00"), ("run.bat", b"@echo off")]));
        assert!(f.serious.iter().any(|s| s.contains("run.bat")), "{f:?}");
        let f = inspect("mods/y.jar", &jar(&[("a/B.class", b"..AppData/Roaming/discord/Local Storage/leveldb..")]));
        assert!(!f.serious.is_empty());
        assert!(inspect("mods/z.exe", b"MZ\x90").serious.len() == 1);
    }

    #[test]
    fn plain_mod_is_clean() {
        let f = inspect("mods/ok.jar", &jar(&[("a/Mod.class", b"\xca\xfe\xba\xbe net/minecraft/world/level/Level"), ("assets/x.png", b"\x89PNG")]));
        assert_eq!(f, Findings::default());
    }
}
