//! Share a pack the CurseForge way: a short code, or a zip.
//!
//! **Code.** A 6-character code that works for 30 minutes. Nothing is uploaded: a signed note under a key
//! made from the code is put on n0's public pkarr relay (the one iroh already uses to find peers). It
//! only says which Obelisk has the pack and until when. Whoever types the code reads the note, reaches
//! that Obelisk directly (share_net.rs), and gets the pack's list of files. Mods that Modrinth or
//! CurseForge know (matched by exact file hash) are downloaded from those sites; the rest come from the
//! sharer. Every file is checked against the list's SHA-1. Files no site knows (a custom jar, scripts)
//! are "custom": the importer is warned and chooses whether to take them.
//!
//! **Zip.** The whole pack (Prism instance format: instance.cfg, mmc-pack.json, minecraft/…) in one
//! zip that Prism, Obelisk and most launchers import.

use crate::{curseforge, history, modrinth};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// No 0/O, 1/I/L: codes are read off screens and typed by hand.
pub const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
pub const CODE_LEN: usize = 6;
pub const LIFETIME_SECS: u64 = 30 * 60;
const RELAY: &str = "https://dns.iroh.link/pkarr";
const TXT_NAME: &str = "_obelisk";

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A fresh random code.
pub fn new_code() -> String {
    let mut out = String::new();
    while out.len() < CODE_LEN {
        for b in iroh::SecretKey::generate().to_bytes() {
            // 248 = 8 * 31: below it every letter is equally likely.
            if b < 248 && out.len() < CODE_LEN { out.push(ALPHABET[(b % 31) as usize] as char) }
        }
    }
    out
}

/// What the user typed or pasted, as a code ("abc 23x" and "ABC-23X" both work).
pub fn normalize(input: &str) -> Result<String, String> {
    let code: String = input.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect();
    if code.len() != CODE_LEN || !code.bytes().all(|b| ALPHABET.contains(&b)) {
        return Err("That isn't a pack code. Codes are 6 letters and digits, like K7MQ2X.".into());
    }
    Ok(code)
}

/// The signing key for a code's note: anyone with the code can find it, nobody without it can.
fn key(code: &str) -> iroh::SecretKey {
    use sha2::{Digest, Sha256};
    let h: [u8; 32] = Sha256::digest(format!("obelisk-pack-code/1:{code}").as_bytes()).into();
    iroh::SecretKey::from_bytes(&h)
}

fn agent() -> Result<ureq::Agent, String> {
    let tls = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
    Ok(ureq::AgentBuilder::new().user_agent("Obelisk/0.1").tls_connector(std::sync::Arc::new(tls))
        .timeout(std::time::Duration::from_secs(30)).build())
}

/// The note a code points to.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    /// The sharing Obelisk's iroh endpoint id.
    pub peer: String,
    /// Unix seconds after which the code is dead.
    pub expires: u64,
}

fn note_text(n: &Note) -> String {
    format!("v=1;peer={};exp={}", n.peer, n.expires)
}

fn parse_note(t: &str) -> Option<Note> {
    let mut peer = None;
    let mut exp = None;
    for kv in t.split(';') {
        match kv.split_once('=') {
            Some(("peer", v)) => peer = Some(v.to_string()),
            Some(("exp", v)) => exp = v.parse().ok(),
            _ => {}
        }
    }
    Some(Note { peer: peer?, expires: exp? })
}

/// Put the code's note on the relay.
pub fn publish(code: &str, note: &Note) -> Result<(), String> {
    let k = key(code);
    let packet = iroh_dns::pkarr::SignedPacket::from_txt_strings(&k, TXT_NAME, [note_text(note)], 60)
        .map_err(|e| format!("couldn't make the code: {e}"))?;
    agent()?.put(&format!("{RELAY}/{}", k.public().to_z32()))
        .send_bytes(&packet.to_relay_payload())
        .map_err(|e| format!("couldn't publish the code (is the internet up?): {e}"))?;
    Ok(())
}

/// Read a code's note. Errors say what the user can do.
pub fn resolve(code: &str) -> Result<Note, String> {
    let k = key(code).public();
    let resp = match agent()?.get(&format!("{RELAY}/{}", k.to_z32())).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Err("No pack has that code. Check it, or ask for a new one (codes last 30 minutes).".into()),
        Err(e) => return Err(format!("couldn't look the code up (is the internet up?): {e}")),
    };
    let mut body = Vec::new();
    resp.into_reader().take(4096).read_to_end(&mut body).map_err(|e| e.to_string())?;
    let packet = iroh_dns::pkarr::SignedPacket::from_relay_payload(&k, &body).map_err(|_| "The code's note is damaged.".to_string())?;
    let note = packet.txt_records(TXT_NAME).iter().find_map(|t| parse_note(t)).ok_or("The code's note is damaged.")?;
    if note.expires <= now_secs() {
        return Err("That code has expired. Ask for a new one (codes last 30 minutes).".into());
    }
    Ok(note)
}

// ---------------------------------------------------------------- the pack's file list

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PackFile {
    /// Relative to the game folder, with forward slashes.
    pub path: String,
    pub size: u64,
    pub sha1: String,
    /// "modrinth" or "curseforge" when that site has this exact file; "" otherwise.
    #[serde(default)]
    pub site: String,
    /// Where the site serves it ("" when the author blocks downloads from other apps: it then comes from the sharer).
    #[serde(default)]
    pub url: String,
    /// Code no site vouches for (an unknown jar, scripts): the importer chooses whether to take these.
    #[serde(default)]
    pub custom: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub v: u32,
    pub name: String,
    /// mmc-pack.json (loader and Minecraft version).
    pub pack: String,
    pub files: Vec<PackFile>,
    /// Config files kept on the sharer's PC because they seem to hold a secret (never sent).
    #[serde(skip)]
    pub private: Vec<String>,
}

/// Folders whose files are game content that sites host.
const CONTENT: [&str; 5] = ["mods", "resourcepacks", "shaderpacks", "global_packs", "datapacks"];
/// Folders that hold code the game runs.
const CODE_DIRS: [&str; 2] = ["kubejs", "scripts"];
const CODE_EXT: [&str; 12] = [".jar", ".exe", ".dll", ".bat", ".cmd", ".ps1", ".vbs", ".sh", ".js", ".zs", ".py", ".class"];

fn is_code(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let first = lower.split('/').next().unwrap_or("");
    CODE_DIRS.contains(&first) || CODE_EXT.iter().any(|e| lower.ends_with(e))
}

/// The pack's files (what a backup tracks, minus personal options.txt), each matched to a site by hash.
/// `mr`/`cf` = None skips that site (offline tests; no CurseForge key).
pub fn manifest(game: &Path, name: &str, mr: Option<&modrinth::Client>, cf: Option<&curseforge::Client>) -> Result<Manifest, String> {
    let inst = game.parent().ok_or("no instance folder")?;
    let pack = fs::read_to_string(inst.join("mmc-pack.json")).map_err(|e| format!("mmc-pack.json: {e}"))?;
    let mut files: Vec<PackFile> = history::current(game, &[])?.into_iter()
        .filter(|f| f.path != "options.txt" && !f.path.starts_with("@instance/"))
        .map(|f| PackFile { path: f.path, size: f.size, sha1: f.hash, ..Default::default() })
        .collect();
    // The sharer's secrets (bot tokens, webhooks, passwords in configs) don't leave this PC.
    let mut private = Vec::new();
    files.retain(|f| {
        let secret = fs::read(game.join(&f.path)).is_ok_and(|b| crate::guard::holds_secret(&f.path, &b));
        if secret { private.push(f.path.clone()) }
        !secret
    });
    let content = |p: &str| CONTENT.contains(&p.split('/').next().unwrap_or("")) && !p.contains("/.index/");
    // Modrinth: exact SHA-1.
    if let Some(mr) = mr {
        let want: Vec<String> = files.iter().filter(|f| content(&f.path)).map(|f| f.sha1.clone()).collect();
        let found = mr.lookup_hashes(&want).unwrap_or_default();
        for f in files.iter_mut() {
            let Some(v) = found.get(&f.sha1) else { continue };
            if let Some(vf) = v.files.iter().find(|vf| vf.hashes.get("sha1") == Some(&f.sha1)) {
                f.site = "modrinth".into();
                f.url = vf.url.clone();
            }
        }
    }
    // CurseForge: fingerprint (needs the sharer's key).
    if let Some(cf) = cf {
        let mut by_fp: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, f) in files.iter().enumerate().filter(|(_, f)| f.site.is_empty() && content(&f.path)) {
            if let Ok(bytes) = fs::read(game.join(&f.path)) { by_fp.entry(curseforge::fingerprint(&bytes)).or_default().push(i) }
        }
        let fps: Vec<u32> = by_fp.keys().copied().collect();
        for chunk in fps.chunks(500) {
            for (fp, _mod_id, file) in cf.match_fingerprints(chunk).unwrap_or_default() {
                for &i in by_fp.get(&fp).into_iter().flatten() {
                    files[i].site = "curseforge".into();
                    files[i].url = file["downloadUrl"].as_str().unwrap_or_default().to_string();
                }
            }
        }
    }
    for f in files.iter_mut() {
        f.custom = f.site.is_empty() && (content(&f.path) || is_code(&f.path));
    }
    Ok(Manifest { v: 1, name: name.to_string(), pack, files, private })
}

/// A path from someone else's list, made safe: only inside the pack folders a backup tracks.
pub fn safe_path(p: &str) -> Option<&str> {
    let first = p.split('/').next().unwrap_or("");
    let ok = !p.contains("..") && !p.contains('\\') && !p.contains(':') && !p.starts_with('/') && !crate::guard::blocked(p)
        && history::TRACKED.contains(&first) && p.len() > first.len() + 1;
    ok.then_some(p)
}

// ---------------------------------------------------------------- installing someone else's pack

/// Make the Prism profile for an incoming pack. Returns (instance id, game folder).
pub fn create_instance(instances: &Path, m: &Manifest) -> Result<(String, PathBuf), String> {
    serde_json::from_str::<serde_json::Value>(&m.pack).map_err(|_| "the pack's loader file is damaged")?;
    let base: String = m.name.chars().map(|c| if c.is_alphanumeric() || " -_()".contains(c) { c } else { '_' }).collect();
    let base = if base.trim().is_empty() { "Shared pack".to_string() } else { base.trim().to_string() };
    let id = (1..).map(|i| if i == 1 { base.clone() } else { format!("{base} ({i})") }).find(|n| !instances.join(n).exists()).unwrap();
    let inst = instances.join(&id);
    let game = inst.join("minecraft");
    fs::create_dir_all(&game).map_err(|e| e.to_string())?;
    fs::write(inst.join("instance.cfg"), format!("InstanceType=OneSix\nname={}\n", m.name.replace(['\r', '\n'], " "))).map_err(|e| e.to_string())?;
    fs::write(inst.join("mmc-pack.json"), &m.pack).map_err(|e| e.to_string())?;
    Ok((id, game))
}

fn sha1_of(bytes: &[u8]) -> String {
    crate::sha1_hex(bytes)
}

/// A download address on Modrinth's or CurseForge's own file servers (the only ones a pack list may point at).
pub fn site_url_ok(url: &str) -> bool {
    ["https://cdn.modrinth.com/", "https://edge.forgecdn.net/", "https://mediafilez.forgecdn.net/"].iter().any(|h| url.starts_with(h))
}

/// Download one file from its site. None = not on a site, or the download failed/didn't match.
pub fn from_site(f: &PackFile) -> Option<Vec<u8>> {
    if !site_url_ok(&f.url) { return None }
    let resp = agent().ok()?.get(&f.url).call().ok()?;
    let mut bytes = Vec::with_capacity(f.size as usize);
    resp.into_reader().take(f.size + 1).read_to_end(&mut bytes).ok()?;
    (sha1_of(&bytes) == f.sha1).then_some(bytes)
}

/// Write a checked file into the new pack.
pub fn place(game: &Path, f: &PackFile, bytes: &[u8]) -> Result<(), String> {
    let rel = safe_path(&f.path).ok_or_else(|| format!("refused an unsafe path: {}", f.path))?;
    if sha1_of(bytes) != f.sha1 {
        return Err(format!("{} didn't match the pack's list", f.path));
    }
    let to = game.join(rel);
    if let Some(d) = to.parent() { fs::create_dir_all(d).map_err(|e| e.to_string())? }
    fs::write(&to, bytes).map_err(|e| format!("{}: {e}", to.display()))
}

// ---------------------------------------------------------------- zip

/// Folders and files a shared zip leaves out: worlds, logs, caches, our own backups.
fn zip_skip(rel: &str) -> bool {
    let first = rel.split('/').next().unwrap_or("");
    [".obelisk", "saves", "logs", "crash-reports", "screenshots", "backups", "cache", ".cache", "jfr", "obelisk-runs",
            "mod-backups", "boundless-backup", ".mixin.out", "debug", "downloads", "dynamic-data-pack-cache", "dynamic-resource-pack-cache",
            "particular_cache", "essential", "voicechat_recordings", "schematics", ".stfolder", "lightning"].contains(&first)
        || first.starts_with("mods-removed")
        || first.ends_with(".log") || first.starts_with("hs_err_pid") || first.contains("stall-")
}

/// Zip the whole pack into `out` (Prism's instance layout). `progress(done, total)` in files.
pub fn export_zip(game: &Path, name: &str, out: &Path, mut progress: impl FnMut(usize, usize)) -> Result<(), String> {
    let inst = game.parent().ok_or("no instance folder")?;
    let mut list: Vec<(String, PathBuf)> = Vec::new();
    fn walk(dir: &Path, rel: &str, list: &mut Vec<(String, PathBuf)>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let r = if rel.is_empty() { n.clone() } else { format!("{rel}/{n}") };
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() || (rel.is_empty() && zip_skip(&r)) { continue }
            if ft.is_dir() { walk(&e.path(), &r, list) } else { list.push((r, e.path())) }
        }
    }
    walk(game, "", &mut list);
    let tmp = out.with_extension("zip.part");
    let file = fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let mut z = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
    let result = (|| -> Result<(), String> {
        let cfg = format!("InstanceType=OneSix\nname={}\n", name.replace(['\r', '\n'], " "));
        z.start_file("instance.cfg", opts).map_err(|e| e.to_string())?;
        z.write_all(cfg.as_bytes()).map_err(|e| e.to_string())?;
        for extra in ["mmc-pack.json", "icon.png"] {
            if let Ok(b) = fs::read(inst.join(extra)) {
                z.start_file(extra, opts).map_err(|e| e.to_string())?;
                z.write_all(&b).map_err(|e| e.to_string())?;
            }
        }
        let total = list.len();
        for (n, (rel, path)) in list.iter().enumerate() {
            // The sharer's secrets stay home (see guard::holds_secret).
            if fs::read(path).is_ok_and(|b| crate::guard::holds_secret(rel, &b)) {
                progress(n + 1, total);
                continue;
            }
            let mut f = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
            z.start_file(format!("minecraft/{rel}"), opts).map_err(|e| e.to_string())?;
            std::io::copy(&mut f, &mut z).map_err(|e| format!("{}: {e}", path.display()))?;
            progress(n + 1, total);
        }
        z.finish().map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    let _ = fs::remove_file(out);
    fs::rename(&tmp, out).map_err(|e| e.to_string())
}

/// A free name for the zip in `dir`: "<Pack>.zip", then "<Pack> (2).zip"...
pub fn zip_name(dir: &Path, name: &str) -> PathBuf {
    let base: String = name.chars().map(|c| if c.is_alphanumeric() || " -_()".contains(c) { c } else { '_' }).collect();
    let base = if base.trim().is_empty() { "Modpack".to_string() } else { base.trim().to_string() };
    (1..).map(|i| if i == 1 { dir.join(format!("{base}.zip")) } else { dir.join(format!("{base} ({i}).zip")) }).find(|p| !p.exists()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let game = t.path().join("Pack/minecraft");
        for (p, c) in [("mods/known.jar", "known"), ("mods/mine.jar", "mine"), ("config/a.toml", "x=1"), ("kubejs/server.js", "go()"),
            ("options.txt", "fov:90"), ("saves/w/level.dat", "w"), ("logs/latest.log", "l")] {
            let f = game.join(p);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, c).unwrap();
        }
        fs::write(t.path().join("Pack/mmc-pack.json"), "{\"components\":[]}").unwrap();
        (t, game)
    }

    #[test]
    fn codes() {
        for _ in 0..50 {
            let c = new_code();
            assert_eq!(c.len(), 6);
            assert_eq!(normalize(&c.to_lowercase()).unwrap(), c);
        }
        assert_eq!(normalize(" k7m-q2x ").unwrap(), "K7MQ2X");
        assert!(normalize("K7MQ20").is_err(), "0 is never in a code");
        assert!(normalize("K7MQ2").is_err());
        assert_ne!(key("AAAAAA").public(), key("AAAAAB").public());
        let n = Note { peer: "abc".into(), expires: 99 };
        assert_eq!(parse_note(&note_text(&n)), Some(n));
    }

    #[test]
    fn manifest_marks_custom_code() {
        let (_t, game) = pack();
        let m = manifest(&game, "Pack", None, None).unwrap();
        let get = |p: &str| m.files.iter().find(|f| f.path == p).cloned();
        assert!(get("options.txt").is_none() && get("saves/w/level.dat").is_none());
        assert!(get("mods/mine.jar").unwrap().custom, "no site knows it");
        assert!(get("kubejs/server.js").unwrap().custom, "scripts run in the game");
        assert!(!get("config/a.toml").unwrap().custom, "settings are not code");
        assert_eq!(get("config/a.toml").unwrap().sha1, crate::sha1_hex(b"x=1"));
    }

    #[test]
    fn install_side_checks_paths_and_hashes() {
        let (t, game) = pack();
        let m = manifest(&game, "Pack", None, None).unwrap();
        let (id, dest) = create_instance(t.path(), &m).unwrap();
        assert_eq!(id, "Pack (2)");
        let f = m.files.iter().find(|f| f.path == "config/a.toml").unwrap();
        place(&dest, f, b"x=1").unwrap();
        assert_eq!(fs::read_to_string(dest.join("config/a.toml")).unwrap(), "x=1");
        assert!(place(&dest, f, b"tampered").is_err());
        let evil = PackFile { path: "../../evil.bat".into(), sha1: crate::sha1_hex(b"x"), ..Default::default() };
        assert!(place(&dest, &evil, b"x").is_err());
        assert!(safe_path("options.txt").is_none() && safe_path("mods/a.jar").is_some());
    }

    #[test]
    fn zip_holds_the_pack_not_the_worlds() {
        let (t, game) = pack();
        let out = zip_name(t.path(), "My Pack");
        export_zip(&game, "My Pack", &out, |_, _| {}).unwrap();
        let mut z = zip::ZipArchive::new(fs::File::open(&out).unwrap()).unwrap();
        let names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).collect();
        for want in ["instance.cfg", "mmc-pack.json", "minecraft/mods/known.jar", "minecraft/config/a.toml", "minecraft/options.txt"] {
            assert!(names.contains(&want.to_string()), "{want} missing from {names:?}");
        }
        assert!(!names.iter().any(|n| n.contains("saves/") || n.contains("logs/")));
        assert_eq!(zip_name(t.path(), "My Pack").file_name().unwrap(), "My Pack (2).zip");
    }

    /// Talks to the real relay: run by hand with `cargo test -- --ignored relay`.
    #[test]
    #[ignore]
    fn relay_round_trip() {
        let code = new_code();
        let n = Note { peer: "test-peer".into(), expires: now_secs() + 120 };
        publish(&code, &n).unwrap();
        assert_eq!(resolve(&code).unwrap(), n);
        assert!(resolve(&new_code()).is_err());
    }
}
