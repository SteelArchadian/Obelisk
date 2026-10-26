//! Shared modpacks: several people's Obelisk keep one pack in step, peer to peer (see `share_net`).
//!
//! Every shared pack has a log of changesets in `<game>/.obelisk/share/log/`. A changeset says
//! which files were set to which content (by hash) or deleted, plus a changelog. The first one is
//! the baseline (the whole pack). File contents live in the backup store, so sharing costs no
//! extra space. Members pull each other's changesets when they're both online; incoming ones wait
//! for review (or apply automatically if the pack is set to). Applying takes a safety backup first.
//!
//! What syncs: mods, configs, scripts, resource/shader/data packs and the loader version. Not
//! worlds, and not options.txt (personal settings).

use crate::history::{self, Change, FileEntry};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    /// iroh endpoint id (public key).
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub last_seen_ms: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Share {
    pub pack_id: String,
    pub name: String,
    /// Shared secret from the invite: proves membership, never sent over the wire.
    pub secret: String,
    pub members: Vec<Member>,
    #[serde(default)]
    pub auto_apply: bool,
    pub created_ms: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub path: String,
    /// New content, or None when the file was deleted.
    pub hash: Option<String>,
    #[serde(default)]
    pub size: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Changeset {
    pub id: String,
    pub author: String,
    pub author_name: String,
    pub at_ms: u64,
    pub label: String,
    pub entries: Vec<Entry>,
    pub changes: Vec<Change>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct State {
    /// Changesets this copy has (authored, applied or skipped).
    applied: Vec<String>,
    /// The shared state as of the last publish/apply: local edits since then are "my changes".
    base: Vec<FileEntry>,
}

fn dir(game: &Path) -> PathBuf {
    game.join(".obelisk").join("share")
}

fn write_json<T: Serialize>(p: &Path, v: &T) -> Result<(), String> {
    fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(&tmp, p).map_err(|e| e.to_string())
}

fn read_json<T: for<'de> Deserialize<'de>>(p: &Path) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(p).ok()?).ok()
}

pub fn get(game: &Path) -> Option<Share> {
    read_json(&dir(game).join("share.json"))
}

pub fn save(game: &Path, s: &Share) -> Result<(), String> {
    write_json(&dir(game).join("share.json"), s)
}

fn state(game: &Path) -> State {
    read_json(&dir(game).join("state.json")).unwrap_or_default()
}

fn save_state(game: &Path, s: &State) -> Result<(), String> {
    write_json(&dir(game).join("state.json"), s)
}

/// Only pack content may arrive from other people: never anything outside these folders.
pub fn allowed(path: &str) -> bool {
    if path == history::PACK_FILE {
        return true;
    }
    if path.contains("..") || path.contains('\\') || path.contains(':') || path.starts_with('/') {
        return false;
    }
    let first = path.split('/').next().unwrap_or("");
    history::TRACKED.contains(&first) && path.len() > first.len() + 1
}

fn shared_files(game: &Path, prev: &[FileEntry]) -> Result<Vec<FileEntry>, String> {
    Ok(history::current(game, prev)?.into_iter().filter(|f| f.path != "options.txt").collect())
}

fn new_id(me: &str) -> String {
    format!("{}-{}", history::now_ms(), &me[..me.len().min(8)])
}

/// Start sharing a pack: writes the baseline changeset (the whole pack as it is now).
pub fn start(game: &Path, name: &str, pack_id: &str, secret: &str, me: &Member) -> Result<Share, String> {
    if get(game).is_some() {
        return Err("this pack is already shared".into());
    }
    let files = shared_files(game, &[])?;
    for f in &files {
        if let Some(p) = history::real(game, &f.path) {
            history::store(game, &p, &f.hash)?;
        }
    }
    let share = Share { pack_id: pack_id.into(), name: name.into(), secret: secret.into(), members: vec![me.clone()], auto_apply: false, created_ms: history::now_ms() };
    let cs = Changeset {
        id: new_id(&me.id), author: me.id.clone(), author_name: me.name.clone(), at_ms: history::now_ms(), label: "Shared the pack".into(),
        entries: files.iter().map(|f| Entry { path: f.path.clone(), hash: Some(f.hash.clone()), size: f.size }).collect(),
        changes: vec![],
    };
    write_changeset(game, &cs)?;
    save(game, &share)?;
    save_state(game, &State { applied: vec![cs.id.clone()], base: files })?;
    Ok(share)
}

/// Stop sharing (the pack itself is untouched).
pub fn stop(game: &Path) -> Result<(), String> {
    match fs::remove_dir_all(dir(game)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

pub fn log_ids(game: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir(game).join("log")).into_iter().flatten().flatten()
        .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").map(str::to_string))
        .collect();
    v.sort();
    v
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() < 80 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub fn read_changeset(game: &Path, id: &str) -> Option<Changeset> {
    valid_id(id).then(|| read_json(&dir(game).join("log").join(format!("{id}.json")))).flatten()
}

/// Save a changeset (ours or one received). Anything touching files outside the pack is refused.
pub fn write_changeset(game: &Path, cs: &Changeset) -> Result<(), String> {
    if !valid_id(&cs.id) {
        return Err("invalid changeset id".into());
    }
    if let Some(bad) = cs.entries.iter().find(|e| !allowed(&e.path) || e.hash.as_ref().is_some_and(|h| h.len() != 40 || !h.chars().all(|c| c.is_ascii_hexdigit()))) {
        return Err(format!("refused a change outside the pack: {}", bad.path));
    }
    write_json(&dir(game).join("log").join(format!("{}.json", cs.id)), cs)
}

pub fn has_blob(game: &Path, hash: &str) -> bool {
    hash.len() == 40 && history::object(game, hash).is_file()
}

pub fn blob_path(game: &Path, hash: &str) -> Option<PathBuf> {
    has_blob(game, hash).then(|| history::object(game, hash))
}

/// Keep a received file's content, after checking it is what its hash says.
pub fn store_blob(game: &Path, hash: &str, bytes: &[u8]) -> Result<(), String> {
    use sha1::{Digest, Sha1};
    if format!("{:x}", Sha1::digest(bytes)) != hash {
        return Err("a received file didn't match its checksum".into());
    }
    let dest = history::object(game, hash);
    fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = dest.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &dest).map_err(|e| e.to_string())
}

/// Hashes every logged changeset refers to (so backup clean-up keeps them).
pub fn referenced(game: &Path) -> HashSet<String> {
    log_ids(game).iter().filter_map(|id| read_changeset(game, id)).flat_map(|c| c.entries).filter_map(|e| e.hash).collect()
}

/// My changes since I last published or applied.
pub fn local_changes(game: &Path) -> Result<Vec<Change>, String> {
    let st = state(game);
    let now = shared_files(game, &st.base)?;
    Ok(history::diff(game, &st.base, &now, true))
}

/// Publish my changes as a changeset friends will pull. None when there's nothing to share.
pub fn publish(game: &Path, me: &Member, label: &str) -> Result<Option<Changeset>, String> {
    let mut st = state(game);
    let now = shared_files(game, &st.base)?;
    let changes = history::diff(game, &st.base, &now, true);
    if changes.is_empty() {
        return Ok(None);
    }
    let old: HashMap<&str, &FileEntry> = st.base.iter().map(|e| (e.path.as_str(), e)).collect();
    let new: HashMap<&str, &FileEntry> = now.iter().map(|e| (e.path.as_str(), e)).collect();
    let mut entries = Vec::new();
    for f in &now {
        if old.get(f.path.as_str()).is_none_or(|o| o.hash != f.hash) {
            if let Some(p) = history::real(game, &f.path) {
                history::store(game, &p, &f.hash)?;
            }
            entries.push(Entry { path: f.path.clone(), hash: Some(f.hash.clone()), size: f.size });
        }
    }
    for o in &st.base {
        if !new.contains_key(o.path.as_str()) {
            entries.push(Entry { path: o.path.clone(), hash: None, size: 0 });
        }
    }
    let cs = Changeset {
        id: new_id(&me.id), author: me.id.clone(), author_name: me.name.clone(), at_ms: history::now_ms(),
        label: if label.trim().is_empty() { String::new() } else { label.trim().to_string() }, entries, changes,
    };
    write_changeset(game, &cs)?;
    st.applied.push(cs.id.clone());
    st.base = now;
    save_state(game, &st)?;
    Ok(Some(cs))
}

/// Changesets from friends that haven't been applied or skipped yet, oldest first.
pub fn incoming(game: &Path) -> Vec<Changeset> {
    let st = state(game);
    let done: HashSet<&str> = st.applied.iter().map(String::as_str).collect();
    let mut v: Vec<Changeset> = log_ids(game).iter().filter(|id| !done.contains(id.as_str())).filter_map(|id| read_changeset(game, id)).collect();
    v.sort_by(|a, b| a.at_ms.cmp(&b.at_ms).then(a.id.cmp(&b.id)));
    v
}

/// Files a changeset needs that we don't have yet.
pub fn missing_blobs(game: &Path, cs: &Changeset) -> Vec<String> {
    cs.entries.iter().filter_map(|e| e.hash.clone()).filter(|h| !has_blob(game, h)).collect()
}

#[derive(Serialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub applied: usize,
    /// Files I had changed myself that a friend's change replaced (my version is in the safety backup).
    pub conflicts: Vec<String>,
}

/// Apply friends' changesets (all incoming ones when `ids` is empty). Takes a safety backup first.
pub fn apply(game: &Path, ids: &[String]) -> Result<Applied, String> {
    let todo: Vec<Changeset> = incoming(game).into_iter().filter(|c| ids.is_empty() || ids.contains(&c.id)).collect();
    if todo.is_empty() {
        return Ok(Applied::default());
    }
    if let Some(missing) = todo.iter().find(|c| !missing_blobs(game, c).is_empty()) {
        return Err(format!("Still downloading the files for {}'s changes. Try again in a moment.", missing.author_name));
    }
    let who: Vec<&str> = todo.iter().map(|c| c.author_name.as_str()).collect::<HashSet<_>>().into_iter().collect();
    history::create(game, "safety", &format!("Before applying changes from {}", who.join(", ")), true)?;

    let mut st = state(game);
    let mut base: HashMap<String, FileEntry> = st.base.drain(..).map(|e| (e.path.clone(), e)).collect();
    let mut out = Applied::default();
    for cs in &todo {
        for e in &cs.entries {
            let dest = history::real(game, &e.path).ok_or("invalid path")?;
            let mine = if dest.is_file() { history::hash_file(&dest).ok() } else { None };
            let shared = base.get(&e.path).map(|b| b.hash.clone());
            if mine != shared && mine != e.hash {
                out.conflicts.push(e.path.clone());
            }
            let entry = e.hash.as_ref().map(|h| FileEntry { path: e.path.clone(), size: e.size, mtime_ms: 0, hash: h.clone() });
            history::put(game, &e.path, entry.as_ref())?;
            match entry {
                Some(fe) => { base.insert(e.path.clone(), fe); }
                None => { base.remove(&e.path); }
            }
        }
        st.applied.push(cs.id.clone());
        out.applied += 1;
    }
    // Re-read sizes/mtimes so the next "my changes" check doesn't rehash everything.
    let mut b: Vec<FileEntry> = base.into_values().collect();
    b.sort_by(|a, c| a.path.cmp(&c.path));
    st.base = shared_files(game, &b)?.into_iter().filter(|f| b.iter().any(|x| x.path == f.path && x.hash == f.hash)).collect();
    // Files that differ from the shared state stay "my changes"; keep their shared version in base.
    for x in b {
        if !st.base.iter().any(|f| f.path == x.path) {
            st.base.push(x);
        }
    }
    st.base.sort_by(|a, c| a.path.cmp(&c.path));
    out.conflicts.sort();
    out.conflicts.dedup();
    save_state(game, &st)?;
    Ok(out)
}

/// Don't apply a friend's changeset (keep my version). It won't be offered again.
pub fn skip(game: &Path, id: &str) -> Result<(), String> {
    let mut st = state(game);
    if !st.applied.iter().any(|a| a == id) {
        st.applied.push(id.to_string());
    }
    save_state(game, &st)
}

/// Remember who's in the pack (merges what a friend told us).
pub fn merge_members(game: &Path, seen: &[Member]) -> Result<(), String> {
    let Some(mut s) = get(game) else { return Ok(()) };
    for m in seen {
        match s.members.iter_mut().find(|x| x.id == m.id) {
            Some(x) => {
                if !m.name.is_empty() { x.name = m.name.clone(); }
                x.last_seen_ms = x.last_seen_ms.max(m.last_seen_ms);
            }
            None => s.members.push(m.clone()),
        }
    }
    save(game, &s)
}

/// Mutual proof of knowing the pack secret, bound to both ends of the connection.
pub fn proof(secret: &str, pack_id: &str, from: &str, to: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(format!("obelisk-share-v1|{secret}|{pack_id}|{from}|{to}").as_bytes()))
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Invite {
    #[serde(rename = "p")]
    pub pack_id: String,
    #[serde(rename = "s")]
    pub secret: String,
    #[serde(rename = "n")]
    pub name: String,
    /// The inviter's endpoint address (id plus how to reach it right now).
    #[serde(rename = "a")]
    pub addr: serde_json::Value,
}

pub fn invite_code(inv: &Invite) -> String {
    use base64::Engine;
    format!("obelisk1.{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(inv).unwrap_or_default()))
}

pub fn parse_invite(code: &str) -> Result<Invite, String> {
    use base64::Engine;
    let body = code.trim().strip_prefix("obelisk1.").ok_or("That isn't an Obelisk invite code.")?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(body).map_err(|_| "The invite code is incomplete. Copy all of it.")?;
    serde_json::from_slice(&bytes).map_err(|_| "The invite code is damaged.".to_string())
}

/// The game folder of the shared pack with this id, if this computer has it.
pub fn find(instances: &Path, pack_id: &str) -> Option<PathBuf> {
    for e in fs::read_dir(instances).ok()?.flatten() {
        for g in ["minecraft", ".minecraft"] {
            let game = e.path().join(g);
            if get(&game).is_some_and(|s| s.pack_id == pack_id) {
                return Some(game);
            }
        }
    }
    None
}

/// Every shared pack here: (instance folder name, game folder, share).
pub fn all(instances: &Path) -> Vec<(String, PathBuf, Share)> {
    let mut v = Vec::new();
    for e in fs::read_dir(instances).into_iter().flatten().flatten() {
        for g in ["minecraft", ".minecraft"] {
            let game = e.path().join(g);
            if let Some(s) = get(&game) {
                v.push((e.file_name().to_string_lossy().to_string(), game, s));
            }
        }
    }
    v
}

/// What a joining friend needs to create the Prism profile: a minimal instance.cfg and the
/// loader/version file.
pub fn instance_files(game: &Path, name: &str) -> Result<(String, String), String> {
    let inst = game.parent().ok_or("no instance folder")?;
    let pack = fs::read_to_string(inst.join("mmc-pack.json")).map_err(|e| e.to_string())?;
    Ok((format!("InstanceType=OneSix\nname={name}\n"), pack))
}

/// Create the Prism profile for a pack we're joining. Returns (instance id, game folder).
pub fn create_instance(instances: &Path, share: &Share, cfg: &str, pack_json: &str) -> Result<(String, PathBuf), String> {
    let base: String = share.name.chars().map(|c| if c.is_alphanumeric() || " -_()".contains(c) { c } else { '_' }).collect();
    let base = if base.trim().is_empty() { "Shared pack".to_string() } else { base.trim().to_string() };
    let id = (1..).map(|i| if i == 1 { base.clone() } else { format!("{base} ({i})") }).find(|n| !instances.join(n).exists()).unwrap();
    let inst = instances.join(&id);
    let game = inst.join("minecraft");
    fs::create_dir_all(&game).map_err(|e| e.to_string())?;
    let cfg: String = cfg.lines().filter(|l| l.starts_with("InstanceType=") || l.starts_with("name=")).map(|l| format!("{l}\n")).collect();
    fs::write(inst.join("instance.cfg"), cfg).map_err(|e| e.to_string())?;
    serde_json::from_str::<serde_json::Value>(pack_json).map_err(|_| "the loader file from your friend is damaged")?;
    fs::write(inst.join("mmc-pack.json"), pack_json).map_err(|e| e.to_string())?;
    save(&game, share)?;
    save_state(&game, &State::default())?;
    Ok((id, game))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me(id: &str, name: &str) -> Member {
        Member { id: id.into(), name: name.into(), last_seen_ms: 0 }
    }

    /// Copy a changeset and its files from one copy of the pack to another (what the network does).
    fn transfer(from: &Path, to: &Path) {
        for id in log_ids(from) {
            if read_changeset(to, &id).is_some() { continue; }
            let cs = read_changeset(from, &id).unwrap();
            for h in missing_blobs(to, &cs) {
                store_blob(to, &h, &fs::read(blob_path(from, &h).unwrap()).unwrap()).unwrap();
            }
            write_changeset(to, &cs).unwrap();
        }
    }

    #[test]
    fn two_friends_stay_in_step() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("A/minecraft");
        fs::create_dir_all(a.join("mods")).unwrap();
        fs::create_dir_all(a.join("config")).unwrap();
        fs::write(a.join("mods/create.jar"), "create v1").unwrap();
        fs::write(a.join("config/create.toml"), "speed=1").unwrap();
        fs::write(a.join("options.txt"), "fov:90").unwrap();
        fs::write(d.path().join("A/mmc-pack.json"), "{\"components\":[]}").unwrap();

        let alex = me("aaaaaaaaaaaa", "Alex");
        let sam = me("bbbbbbbbbbbb", "Sam");
        let share = start(&a, "Pack", "p1", "s3cret", &alex).unwrap();

        // Sam joins: profile created, baseline pulled and applied.
        let instances = d.path().join("B-instances");
        fs::create_dir_all(&instances).unwrap();
        let (cfg, pj) = instance_files(&a, "Pack").unwrap();
        let (_, b) = create_instance(&instances, &share, &cfg, &pj).unwrap();
        transfer(&a, &b);
        assert_eq!(incoming(&b).len(), 1);
        apply(&b, &[]).unwrap();
        assert_eq!(fs::read_to_string(b.join("mods/create.jar")).unwrap(), "create v1");
        assert!(!b.join("options.txt").exists(), "personal settings don't sync");
        assert!(local_changes(&b).unwrap().is_empty());

        // Alex updates a mod and a config; Sam tweaks the same config meanwhile.
        fs::remove_file(a.join("mods/create.jar")).unwrap();
        fs::write(a.join("mods/create2.jar"), "create v2").unwrap();
        fs::write(a.join("config/create.toml"), "speed=5").unwrap();
        let cs = publish(&a, &alex, "Faster Create").unwrap().unwrap();
        assert_eq!(cs.entries.len(), 3);
        fs::write(b.join("config/create.toml"), "speed=2").unwrap();
        transfer(&a, &b);
        let r = apply(&b, &[]).unwrap();
        assert_eq!(r.conflicts, vec!["config/create.toml".to_string()]);
        assert_eq!(fs::read_to_string(b.join("config/create.toml")).unwrap(), "speed=5");
        assert!(b.join("mods/create2.jar").is_file() && !b.join("mods/create.jar").exists());
        assert!(history::list(&b).iter().any(|x| x.kind == "safety"), "Sam's version is in a safety backup");

        // Sam adds a mod; Alex pulls it and can skip it.
        fs::write(b.join("mods/jei.jar"), "jei").unwrap();
        publish(&b, &sam, "").unwrap().unwrap();
        transfer(&b, &a);
        let inc = incoming(&a);
        assert_eq!(inc.len(), 1);
        skip(&a, &inc[0].id).unwrap();
        assert!(incoming(&a).is_empty() && !a.join("mods/jei.jar").exists());
    }

    #[test]
    fn refuses_files_outside_the_pack() {
        let d = tempfile::tempdir().unwrap();
        let cs = Changeset { id: "1-x".into(), author: "x".into(), author_name: "x".into(), at_ms: 1, label: String::new(),
            entries: vec![Entry { path: "../../evil.bat".into(), hash: Some("a".repeat(40)), size: 1 }], changes: vec![] };
        assert!(write_changeset(d.path(), &cs).is_err());
        for bad in ["saves/World/level.dat", "options.txt", "mods", "C:/x", "config\\..\\x"] {
            assert!(!allowed(bad), "{bad}");
        }
        assert!(allowed("mods/create.jar") && allowed("config/a/b.toml") && allowed(history::PACK_FILE));
    }

    #[test]
    fn invite_round_trip() {
        let inv = Invite { pack_id: "p".into(), secret: "s".into(), name: "My Pack".into(), addr: serde_json::json!({"id": "abc"}) };
        assert_eq!(parse_invite(&invite_code(&inv)).unwrap(), inv);
        assert!(parse_invite("hello").is_err());
        assert_ne!(proof("s", "p", "a", "b"), proof("s", "p", "b", "a"));
    }
}
