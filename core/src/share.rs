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
    /// Who made the pack collaborative: the only one who can remove people. Older shares fill it in from
    /// the baseline's author (see `owner`).
    #[serde(default)]
    pub owner: String,
    /// People the owner removed, each with the owner's signature. They're refused, and told so.
    #[serde(default)]
    pub removed: Vec<Removal>,
    /// Bumped with each removal, when the pack moves to a new secret (the removed person knows the old one).
    #[serde(default)]
    pub epoch: u32,
    /// Earlier secrets, so members who haven't heard about a removal yet can still connect and catch up.
    #[serde(default)]
    pub old_secrets: Vec<String>,
}

/// The owner's signed note that someone was removed. It also commits to the pack's next secret (by hash), so
/// a member only switches to a new secret the owner actually chose.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Removal {
    pub pack_id: String,
    pub id: String,
    pub name: String,
    pub by: String,
    pub by_name: String,
    pub at_ms: u64,
    pub epoch: u32,
    pub secret_hash: String,
    pub sig: String,
}

impl Removal {
    fn message(&self) -> Vec<u8> {
        format!("obelisk-remove-v1|{}|{}|{}|{}|{}", self.pack_id, self.id, self.epoch, self.secret_hash, self.at_ms).into_bytes()
    }

    /// Signed by `owner` (an endpoint id) for this pack.
    pub fn valid(&self, pack_id: &str, owner: &str) -> bool {
        let (Ok(key), Ok(sig)) = (owner.parse::<iroh::PublicKey>(), hex_bytes::<64>(&self.sig)) else { return false };
        self.pack_id == pack_id && self.by == owner && key.verify(&self.message(), &iroh::Signature::from_bytes(&sig)).is_ok()
    }
}

fn hex_bytes<const N: usize>(s: &str) -> Result<[u8; N], ()> {
    if s.len() != N * 2 { return Err(()) }
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2).ok_or(())?, 16).map_err(|_| ())?;
    }
    Ok(out)
}

pub fn secret_hash(secret: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(format!("obelisk-secret-v1|{secret}").as_bytes()))
}

/// One member's wish list for the pack: projects they'd like added, to talk over first.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct WishList {
    pub name: String,
    pub updated_ms: u64,
    /// `{ hit, addedMs }` as the app stores them; the core only passes them along.
    pub items: Vec<serde_json::Value>,
}

/// Shown on a pack after its owner removed us (the pack itself stays as it was).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Ended {
    pub by_name: String,
    pub at_ms: u64,
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
    /// Only in 0.2.26: blocked files were listed here. They move to held.rs when read.
    #[serde(default, skip_serializing)]
    held: Vec<OldHeld>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct OldHeld {
    path: String,
    hash: String,
    author_name: String,
    reasons: Vec<String>,
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
    let mut st: State = read_json(&dir(game).join("state.json")).unwrap_or_default();
    if !st.held.is_empty() {
        for h in st.held.drain(..) {
            if let Some(bytes) = blob_path(game, &h.hash).and_then(|b| fs::read(b).ok()) {
                let _ = crate::held::put(game, &h.path, &bytes, &h.author_name, h.reasons);
            }
        }
        let _ = save_state(game, &st);
    }
    st
}

fn save_state(game: &Path, s: &State) -> Result<(), String> {
    write_json(&dir(game).join("state.json"), s)
}

/// Only pack content may arrive from other people: never anything outside these folders.
pub fn allowed(path: &str) -> bool {
    if path == history::PACK_FILE {
        return true;
    }
    if path.contains("..") || path.contains('\\') || path.contains(':') || path.starts_with('/') || crate::guard::blocked(path) {
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
    let share = Share { pack_id: pack_id.into(), name: name.into(), secret: secret.into(), members: vec![me.clone()], auto_apply: false,
        created_ms: history::now_ms(), owner: me.id.clone(), removed: vec![], epoch: 0, old_secrets: vec![] };
    let cs = Changeset {
        id: new_id(&me.id), author: me.id.clone(), author_name: me.name.clone(), at_ms: history::now_ms(), label: "Shared the pack".into(),
        entries: files.iter().map(|f| Entry { path: f.path.clone(), hash: Some(f.hash.clone()), size: f.size }).collect(),
        changes: vec![],
    };
    write_changeset(game, &cs)?;
    save(game, &share)?;
    save_state(game, &State { applied: vec![cs.id.clone()], base: files, ..Default::default() })?;
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
    let mut st = state(game);
    unheld(game, &mut st);
    let now = shared_files(game, &st.base)?;
    Ok(history::diff(game, &st.base, &now, true))
}

/// Leave blocked files (held.rs) out of base, so their absence here isn't sent to friends as a deletion.
fn unheld(game: &Path, st: &mut State) {
    let held: HashSet<String> = crate::held::list(game).into_iter().map(|h| h.path).collect();
    st.base.retain(|b| !held.contains(&b.path));
}

/// Publish my changes as a changeset friends will pull. None when there's nothing to share.
pub fn publish(game: &Path, me: &Member, label: &str) -> Result<Option<Changeset>, String> {
    let mut st = state(game);
    let held: HashSet<String> = crate::held::list(game).into_iter().map(|h| h.path).collect();
    let kept: Vec<FileEntry> = st.base.iter().filter(|b| held.contains(&b.path)).cloned().collect();
    unheld(game, &mut st);
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
    let kept: Vec<FileEntry> = kept.into_iter().filter(|k| !now.iter().any(|n| n.path == k.path)).collect();
    st.base = now;
    st.base.extend(kept);
    st.base.sort_by(|a, c| a.path.cmp(&c.path));
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
    /// Friends' files Obelisk blocked (see held).
    pub blocked: Vec<String>,
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
    let blocked = vet_incoming(game, &todo)?;
    let who: Vec<&str> = todo.iter().map(|c| c.author_name.as_str()).collect::<HashSet<_>>().into_iter().collect();
    history::create(game, "safety", &format!("Before applying changes from {}", who.join(", ")), true)?;

    let mut st = state(game);
    let mut base: HashMap<String, FileEntry> = st.base.drain(..).map(|e| (e.path.clone(), e)).collect();
    let mut out = Applied::default();
    for cs in &todo {
        for e in &cs.entries {
            let was_held = crate::held::list(game).iter().any(|h| h.path == e.path);
            crate::held::forget(game, &e.path)?;
            if let Some(reasons) = e.hash.as_ref().and_then(|h| blocked.get(&(e.path.clone(), h.clone()))) {
                // Not written: kept aside, and the window asks the user about it (held.rs).
                let hash = e.hash.clone().unwrap_or_default();
                let bytes = fs::read(blob_path(game, &hash).ok_or("invalid file id")?).map_err(|err| format!("{}: {err}", e.path))?;
                crate::held::put(game, &e.path, &bytes, &cs.author_name, reasons.clone())?;
                base.insert(e.path.clone(), FileEntry { path: e.path.clone(), size: e.size, mtime_ms: 0, hash });
                out.blocked.push(e.path.clone());
                continue;
            }
            let dest = history::real(game, &e.path).ok_or("invalid path")?;
            let mine = if dest.is_file() { history::hash_file(&dest).ok() } else { None };
            let shared = base.get(&e.path).map(|b| b.hash.clone());
            if mine != shared && mine != e.hash && !was_held {
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
    let held: HashSet<String> = crate::held::list(game).into_iter().map(|h| h.path).collect();
    let mut b: Vec<FileEntry> = base.into_values().collect();
    b.sort_by(|a, c| a.path.cmp(&c.path));
    st.base = shared_files(game, &b)?.into_iter().filter(|f| b.iter().any(|x| x.path == f.path && x.hash == f.hash)).collect();
    // Files that differ from the shared state stay "my changes"; keep their shared version in base.
    // Blocked files keep their shared version too (they never count as my changes).
    for x in b {
        if held.contains(&x.path) {
            st.base.retain(|f| f.path != x.path);
            st.base.push(x);
        } else if !st.base.iter().any(|f| f.path == x.path) {
            st.base.push(x);
        }
    }
    st.base.sort_by(|a, c| a.path.cmp(&c.path));
    out.blocked.sort();
    out.blocked.dedup();
    out.blocked.retain(|p| held.contains(p));
    out.conflicts.sort();
    out.conflicts.dedup();
    save_state(game, &st)?;
    Ok(out)
}

/// Friends' files are checked before they land, the same way as a share code's custom files: code is opened
/// and blocked on signs of malware (guard.rs), and Windows Defender scans everything that would arrive.
/// Returns the blocked files ((path, hash) → why); a Defender threat stops the whole apply.
fn vet_incoming(game: &Path, todo: &[Changeset]) -> Result<HashMap<(String, String), Vec<String>>, String> {
    let mut blocked = HashMap::new();
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    for cs in todo {
        for e in &cs.entries {
            let Some(hash) = &e.hash else { continue };
            if !allowed(&e.path) {
                return Err(format!("{}'s changes include {}, which Obelisk never accepts from another PC.", cs.author_name, e.path));
            }
            let blob = blob_path(game, hash).ok_or("invalid file id")?;
            let bytes = fs::read(&blob).map_err(|err| format!("{}: {err}", e.path))?;
            if crate::guard::risky(&e.path) {
                let f = crate::guard::inspect(&e.path, &bytes);
                if !f.serious.is_empty() {
                    blocked.insert((e.path.clone(), hash.clone()), f.serious);
                    continue;
                }
            }
            let to = dir.path().join(&e.path);
            if let Some(d) = to.parent() { fs::create_dir_all(d).map_err(|e| e.to_string())? }
            fs::write(&to, &bytes).map_err(|e| e.to_string())?;
        }
    }
    if let crate::guard::Scan::Threat(t) = crate::guard::defender_scan(dir.path()) {
        return Err(format!("Not applying these changes: Windows Defender found a threat in them ({t})."));
    }
    Ok(blocked)
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
    for m in seen.iter().filter(|m| !s.removed.iter().any(|r| r.id == m.id)) {
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
    // Only real profiles (Prism lists a folder only with its instance.cfg); a join that failed half-way doesn't count.
    for e in fs::read_dir(instances).ok()?.flatten().filter(|e| e.path().join("instance.cfg").is_file()) {
        for g in ["minecraft", ".minecraft"] {
            let game = e.path().join(g);
            if get(&game).is_some_and(|s| s.pack_id == pack_id) {
                return Some(game);
            }
        }
    }
    None
}

/// What a join or import that failed half-way left behind: folders Obelisk made (they hold its `.obelisk`
/// folder) that aren't profiles, because removing them didn't finish (a file was in use).
pub fn half_made(instances: &Path) -> Vec<PathBuf> {
    fs::read_dir(instances).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.is_dir() && !p.join("instance.cfg").exists() && ["minecraft", ".minecraft"].iter().any(|g| p.join(g).join(".obelisk").is_dir()))
        .collect()
}

/// Remove a profile a join or import was making when it failed. A file still in use (an antivirus scanning a
/// jar) can stop it, so it tries a few times; what's left is half_made's, and the app moves it to the Recycle Bin.
pub fn discard(inst: &Path) {
    for _ in 0..5 {
        if fs::remove_dir_all(inst).is_ok() || !inst.exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
}

/// Every shared pack here: (instance folder name, game folder, share).
pub fn all(instances: &Path) -> Vec<(String, PathBuf, Share)> {
    let mut v = Vec::new();
    for e in fs::read_dir(instances).into_iter().flatten().flatten().filter(|e| e.path().join("instance.cfg").is_file()) {
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

/// The pack's owner: the stored one, or for shares made before owners were kept, the baseline's author.
pub fn owner(game: &Path, s: &Share) -> String {
    if !s.owner.is_empty() {
        return s.owner.clone();
    }
    log_ids(game).first().and_then(|id| read_changeset(game, id)).map(|c| c.author).unwrap_or_default()
}

/// Every secret a member may still be using, newest first.
pub fn secrets(s: &Share) -> Vec<String> {
    std::iter::once(s.secret.clone()).chain(s.old_secrets.iter().rev().cloned()).collect()
}

pub fn is_removed(s: &Share, id: &str) -> bool {
    s.removed.iter().any(|r| r.id == id)
}

/// The owner removes someone: they're refused from now on and told so, and the pack moves to a new secret that
/// only the remaining members get. Returns the removal (already saved).
pub fn remove_member(game: &Path, key: &iroh::SecretKey, my_name: &str, id: &str, new_secret: &str) -> Result<Removal, String> {
    let mut s = get(game).ok_or("this pack isn't collaborative")?;
    let me = key.public().to_string();
    if owner(game, &s) != me {
        return Err("Only the person who made the pack collaborative can remove people.".into());
    }
    if id == me {
        return Err("You can't remove yourself. Turn collaboration off instead.".into());
    }
    let name = s.members.iter().find(|m| m.id == id).map(|m| m.name.clone()).ok_or("They aren't in this pack.")?;
    let mut r = Removal {
        pack_id: s.pack_id.clone(), id: id.into(), name, by: me.clone(), by_name: my_name.into(), at_ms: history::now_ms(),
        epoch: s.epoch + 1, secret_hash: secret_hash(new_secret), sig: String::new(),
    };
    r.sig = key.sign(&r.message()).to_bytes().iter().map(|b| format!("{b:02x}")).collect();
    s.owner = me;
    s.old_secrets.push(std::mem::replace(&mut s.secret, new_secret.into()));
    s.epoch = r.epoch;
    s.members.retain(|m| m.id != id);
    s.removed.push(r.clone());
    save(game, &s)?;
    drop_wishes(game, id)?;
    Ok(r)
}

/// Take in removals a member passed along (only ones the owner signed). Returns true when one of them is us,
/// in which case the pack has stopped being collaborative here and shows who removed us.
pub fn merge_removals(game: &Path, me: &str, rs: &[Removal]) -> Result<bool, String> {
    let Some(mut s) = get(game) else { return Ok(false) };
    let owner = owner(game, &s);
    let mut changed = false;
    for r in rs.iter().filter(|r| r.valid(&s.pack_id, &owner)) {
        if r.id == me {
            end(game, r)?;
            return Ok(true);
        }
        if !is_removed(&s, &r.id) {
            s.members.retain(|m| m.id != r.id);
            s.removed.push(r.clone());
            drop_wishes(game, &r.id)?;
            changed = true;
        }
    }
    if s.owner.is_empty() && !owner.is_empty() {
        s.owner = owner;
        changed = true;
    }
    if changed { save(game, &s)?; }
    Ok(false)
}

/// Switch to a newer secret a member handed us, if the owner's newest signed removal names it.
pub fn adopt_secret(game: &Path, secret: &str) -> Result<bool, String> {
    let Some(mut s) = get(game) else { return Ok(false) };
    let owner = owner(game, &s);
    let Some(r) = s.removed.iter().filter(|r| r.valid(&s.pack_id, &owner)).max_by_key(|r| r.epoch) else { return Ok(false) };
    if r.epoch <= s.epoch || r.secret_hash != secret_hash(secret) {
        return Ok(false);
    }
    s.epoch = r.epoch;
    s.old_secrets.push(std::mem::replace(&mut s.secret, secret.into()));
    save(game, &s)?;
    Ok(true)
}

fn ended_path(game: &Path) -> PathBuf {
    game.join(".obelisk").join("share-ended.json")
}

/// We were removed: stop collaborating (files stay as they are) and leave a note for the pack's page.
pub fn end(game: &Path, r: &Removal) -> Result<(), String> {
    write_json(&ended_path(game), &Ended { by_name: r.by_name.clone(), at_ms: r.at_ms })?;
    stop(game)
}

pub fn ended(game: &Path) -> Option<Ended> {
    read_json(&ended_path(game))
}

pub fn dismiss_ended(game: &Path) -> Result<(), String> {
    match fs::remove_file(ended_path(game)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// Everyone's wish lists for this pack, by member id.
pub fn wishes(game: &Path) -> HashMap<String, WishList> {
    read_json(&dir(game).join("wishes.json")).unwrap_or_default()
}

/// Replace my own wish list.
pub fn set_wishes(game: &Path, me: &Member, items: Vec<serde_json::Value>) -> Result<(), String> {
    let mut all = wishes(game);
    all.insert(me.id.clone(), WishList { name: me.name.clone(), updated_ms: history::now_ms(), items });
    write_json(&dir(game).join("wishes.json"), &all)
}

/// Take in the lists a member passed along: each person's newest list wins (only they change it). Lists of
/// people who aren't (or are no longer) members are ignored.
pub fn merge_wishes(game: &Path, theirs: HashMap<String, WishList>) -> Result<(), String> {
    let Some(s) = get(game) else { return Ok(()) };
    let mut all = wishes(game);
    let mut changed = false;
    for (id, w) in theirs {
        if is_removed(&s, &id) || !s.members.iter().any(|m| m.id == id) {
            continue;
        }
        if all.get(&id).is_none_or(|mine| w.updated_ms > mine.updated_ms) {
            all.insert(id, w);
            changed = true;
        }
    }
    if changed { write_json(&dir(game).join("wishes.json"), &all)?; }
    Ok(())
}

fn drop_wishes(game: &Path, id: &str) -> Result<(), String> {
    let mut all = wishes(game);
    if all.remove(id).is_some() {
        write_json(&dir(game).join("wishes.json"), &all)?;
    }
    Ok(())
}

/// The whole change log, newest first, without file lists: who changed what, and when.
pub fn history_log(game: &Path) -> Vec<Changeset> {
    let mut v: Vec<Changeset> = log_ids(game).iter().rev().filter_map(|id| read_changeset(game, id)).collect();
    for c in &mut v { c.entries.clear(); }
    v
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
    fn blocked_files_wait_for_the_user() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("A/minecraft");
        fs::create_dir_all(a.join("mods")).unwrap();
        fs::write(a.join("mods/create.jar"), "create v1").unwrap();
        fs::write(d.path().join("A/mmc-pack.json"), "{\"components\":[]}").unwrap();
        let mut jar = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        jar.start_file("x/Accounts.class", zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut jar, b"\xca\xfe\xba\xbe .minecraft/microsoft_accounts.json").unwrap();
        fs::write(a.join("mods/essential.jar"), jar.finish().unwrap().into_inner()).unwrap();
        let alex = me("aaaaaaaaaaaa", "Alex");
        let sam = me("bbbbbbbbbbbb", "Sam");
        let share = start(&a, "Pack", "p1", "s3cret", &alex).unwrap();

        let instances = d.path().join("B-instances");
        fs::create_dir_all(&instances).unwrap();
        let (cfg, pj) = instance_files(&a, "Pack").unwrap();
        let (_, b) = create_instance(&instances, &share, &cfg, &pj).unwrap();
        transfer(&a, &b);
        let r = apply(&b, &[]).unwrap();
        assert_eq!(r.blocked, vec!["mods/essential.jar".to_string()]);
        assert!(b.join("mods/create.jar").is_file() && !b.join("mods/essential.jar").exists());
        let h = crate::held::list(&b);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].from, "Alex");
        assert!(h[0].reasons[0].contains("Microsoft accounts"), "{:?}", h[0].reasons);
        // Its absence isn't sent back to friends as a deletion.
        assert!(local_changes(&b).unwrap().is_empty());
        fs::write(b.join("mods/jei.jar"), "jei").unwrap();
        let cs = publish(&b, &sam, "").unwrap().unwrap();
        assert!(cs.entries.iter().all(|e| e.path == "mods/jei.jar"), "{:?}", cs.entries);
        assert_eq!(crate::held::list(&b).len(), 1);

        // The user allows it.
        crate::held::allow(&b, "mods/essential.jar").unwrap();
        assert!(b.join("mods/essential.jar").is_file() && crate::held::list(&b).is_empty());
        assert!(local_changes(&b).unwrap().is_empty());
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

    #[test]
    fn owner_removes_a_member_and_the_rest_rekey() {
        let d = tempfile::tempdir().unwrap();
        let owner_key = iroh::SecretKey::from_bytes(&[1; 32]);
        let (o, m, x) = (owner_key.public().to_string(), iroh::SecretKey::from_bytes(&[2; 32]).public().to_string(), iroh::SecretKey::from_bytes(&[3; 32]).public().to_string());
        let game = |n: &str| { let g = d.path().join(n).join("minecraft"); fs::create_dir_all(g.join("mods")).unwrap(); g };
        let (go, gm, gx) = (game("O"), game("M"), game("X"));
        let share = start(&go, "Pack", "p1", "old", &me(&o, "Olive")).unwrap();
        for (g, id) in [(&gm, &m), (&gx, &x)] {
            let mut s = share.clone();
            s.owner = String::new(); // as an older share would have it: worked out from the baseline
            s.members.push(me(id, ""));
            save(g, &s).unwrap();
            transfer(&go, g);
        }
        merge_members(&go, &[me(&m, "Mia"), me(&x, "Xan")]).unwrap();
        set_wishes(&gx, &me(&x, "Xan"), vec![serde_json::json!({ "hit": { "project_id": "AANobbMI" } })]).unwrap();
        merge_wishes(&go, wishes(&gx)).unwrap();
        assert_eq!(wishes(&go).len(), 1);

        // Only the owner can remove, and a forged removal is ignored.
        assert!(remove_member(&gm, &iroh::SecretKey::from_bytes(&[2; 32]), "Mia", &x, "new").is_err());
        let r = remove_member(&go, &owner_key, "Olive", &x, "new").unwrap();
        let s = get(&go).unwrap();
        assert_eq!((s.secret.as_str(), s.epoch), ("new", 1));
        assert!(!s.members.iter().any(|mm| mm.id == x) && wishes(&go).is_empty());
        merge_members(&go, &[me(&x, "Xan")]).unwrap();
        assert!(!get(&go).unwrap().members.iter().any(|mm| mm.id == x), "a removed member isn't added back");
        let mut forged = r.clone();
        forged.id = o.clone();
        assert!(!merge_removals(&gm, &m, &[forged]).unwrap());

        // A member hears about it, and takes the new secret only when it's the one the owner signed for.
        assert!(!merge_removals(&gm, &m, std::slice::from_ref(&r)).unwrap());
        assert!(!adopt_secret(&gm, "made-up").unwrap());
        assert!(adopt_secret(&gm, "new").unwrap());
        let sm = get(&gm).unwrap();
        assert_eq!((sm.secret.as_str(), sm.old_secrets.clone(), sm.owner.clone()), ("new", vec!["old".to_string()], o.clone()));

        // The removed one stops collaborating and is told who removed them.
        assert!(merge_removals(&gx, &x, &[r]).unwrap());
        assert!(get(&gx).is_none());
        assert_eq!(ended(&gx).unwrap().by_name, "Olive");
        dismiss_ended(&gx).unwrap();
        assert!(ended(&gx).is_none());
    }
}
