//! The network side of shared packs: Obelisk talking to Obelisk directly over iroh (encrypted QUIC,
//! dialed by public key, through home routers; n0's public relays are used only when a direct
//! path can't be made, and they can't read the traffic).
//!
//! Each request is one bi-directional stream: a JSON request, then a JSON header (plus raw file
//! bytes for blobs). The first request on a connection must be Hello, where both sides prove they
//! know the pack's secret without sending it. Nothing is served before that.

use crate::guard::{self, Findings, Scan};
use crate::packcode::{self, Manifest, PackFile};
use crate::share::{self, Changeset, Invite, Member, Removal, Share, WishList};
use std::collections::HashMap;
use iroh::endpoint::{presets, Connection};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const ALPN: &[u8] = b"obelisk/share/1";
/// Pack codes (packcode.rs): whoever has a live code gets that pack's file list and files.
pub const ALPN_PACK: &[u8] = b"obelisk/pack/1";
const MAX_JSON: usize = 32 << 20;
const WRONG_KEY: &str = "wrong pack key";
const MAX_BLOB: usize = 600 << 20;

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "t", rename_all = "camelCase")]
enum Req {
    Hello { pack: String, name: String, proof: String },
    Log,
    Changeset { id: String },
    Blob { hash: String },
    Instance,
    /// Everyone's wish lists (older Obelisk answers "bad request", which is skipped).
    Wishes,
}

#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
struct Resp {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<serde_json::Value>,
    /// For blobs: how many raw bytes follow the header.
    #[serde(default)]
    size: u64,
}

fn now_ms() -> u64 {
    crate::history::now_ms()
}

/// The running peer: our endpoint plus the router answering friends.
pub struct Net {
    pub endpoint: Endpoint,
    _router: Router,
    instances: PathBuf,
    name: Arc<Mutex<String>>,
    offers: Offers,
}

/// A pack offered under a code, until `expires` (unix seconds).
#[derive(Debug, Clone)]
struct Offer {
    game: PathBuf,
    manifest: Arc<Manifest>,
    expires: u64,
    stats: Arc<Mutex<OfferStatus>>,
}

/// How a code's sharing is going, for the sharer's screen: friends connected now, files sent, when the last
/// one went (unix ms), and friends whose import finished.
#[derive(Serialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct OfferStatus {
    pub connected: usize,
    pub sent: usize,
    pub last_ms: u64,
    pub finished: usize,
}

type Offers = Arc<Mutex<HashMap<String, Offer>>>;

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "t", rename_all = "camelCase")]
enum PackReq {
    Manifest { code: String },
    File { code: String, path: String },
}

/// A file from the sharer's PC that would run as code, with what looking inside it found.
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CustomFile {
    pub path: String,
    pub size: u64,
    pub findings: Findings,
}

/// What an incoming pack holds, for the "custom files" question before installing. Worked out by the
/// receiving side itself: the sender's list is never taken at its word about what a file is.
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PackPreview {
    pub name: String,
    pub files: usize,
    pub bytes: u64,
    /// Files Modrinth or CurseForge vouch for (downloaded from them, or matched by their fingerprint).
    pub known: usize,
    /// Code no site vouches for: installed only if the user chooses.
    pub custom: Vec<CustomFile>,
    /// Never installed: Windows programs and scripts, or code with signs of malware.
    pub refused: Vec<CustomFile>,
    /// Windows Defender's verdict on the custom files.
    pub scan: Scan,
    /// The sharer's Java, memory and JVM arguments (vetted), for the friend to take or leave.
    pub game: Option<packcode::GameSettings>,
    /// A Collaborate code: the invite to join with (then nothing else here is filled in).
    pub invite: Option<String>,
}

/// How an import went: the new profile, what was left out and why, and Defender's verdict on the result.
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub id: String,
    pub left_out: Vec<String>,
    pub scan: Scan,
}

/// The receiving side's view of a pack's code files (see `Net::vet`).
struct Vetting {
    known: usize,
    custom: Vec<CustomFile>,
    refused: Vec<CustomFile>,
    scan: Scan,
    /// Bytes already fetched from the sharer and checked, by path.
    fetched: HashMap<String, Vec<u8>>,
}

/// Progress of an incoming pack: what it's doing ("checking" the sharer's files, "installing", "scanning" with
/// Windows Defender), files done of total, and the one being worked on.
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ImportProgress {
    pub step: &'static str,
    pub done: usize,
    pub total: usize,
    pub file: String,
}

type Progress = dyn Fn(ImportProgress) + Send + Sync;

/// The person stopped the pack being checked or installed (Import → Stop).
static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Import → Stop: the check or install under way ends at its next step (a half-made profile is removed).
pub fn stop_incoming() {
    STOP.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn stopped() -> Result<(), String> {
    if STOP.load(std::sync::atomic::Ordering::SeqCst) { Err("Stopped.".into()) } else { Ok(()) }
}

/// Defender scans `dir`, telling the screen how long it's been at it. Import → Stop ends it.
async fn scan_watched(dir: std::path::PathBuf, files: usize, what: &'static str, progress: &Progress) -> Result<Scan, String> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<u64>();
    let job = tokio::task::spawn_blocking(move || guard::defender_scan_watched(&dir, |s| { let _ = tx.send(s); }, || stopped().is_err()));
    let label = |s: u64| format!("Windows Defender is scanning the {what} ({files}) · {}:{:02}", s / 60, s % 60);
    progress(ImportProgress { step: "scanning", done: 0, total: files, file: label(0) });
    tokio::pin!(job);
    loop {
        tokio::select! {
            r = &mut job => return r.map_err(|e| e.to_string()),
            Some(s) = rx.recv() => progress(ImportProgress { step: "scanning", done: 0, total: files, file: label(s) }),
        }
    }
}

/// A sharer that goes quiet this long (asleep, Obelisk closed, connection gone) is given up on.
const STALL: Duration = Duration::from_secs(45);

impl Net {
    pub async fn start(key: [u8; 32], instances: PathBuf, name: String) -> Result<Net, String> {
        let endpoint = Endpoint::builder(presets::N0).secret_key(SecretKey::from_bytes(&key)).bind().await.map_err(|e| e.to_string())?;
        let name = Arc::new(Mutex::new(name));
        let handler = Handler { instances: instances.clone(), name: name.clone(), me: endpoint.id() };
        let offers: Offers = Default::default();
        let router = Router::builder(endpoint.clone()).accept(ALPN, handler).accept(ALPN_PACK, PackHandler { offers: offers.clone() }).spawn();
        Ok(Net { endpoint, _router: router, instances, name, offers })
    }

    pub fn id(&self) -> String {
        self.endpoint.id().to_string()
    }

    pub fn set_name(&self, name: &str) {
        *self.name.lock().unwrap() = name.to_string();
    }

    fn me(&self) -> Member {
        Member { id: self.id(), name: self.name.lock().unwrap().clone(), last_seen_ms: now_ms() }
    }

    /// Our address right now (waits briefly for the relay connection, so invites work at once).
    pub async fn addr(&self) -> EndpointAddr {
        let _ = tokio::time::timeout(Duration::from_secs(8), self.endpoint.online()).await;
        self.endpoint.addr()
    }

    /// Start sharing a pack and return its invite code.
    pub async fn share(&self, game: &Path, name: &str) -> Result<String, String> {
        let s = match share::get(game) {
            Some(s) => s,
            None => {
                let pack = hex(&SecretKey::generate().to_bytes()[..16]);
                let secret = hex(&SecretKey::generate().to_bytes());
                share::start(game, name, &pack, &secret, &self.me())?
            }
        };
        self.invite(&s).await
    }

    pub async fn invite(&self, s: &Share) -> Result<String, String> {
        let addr = serde_json::to_value(self.addr().await).map_err(|e| e.to_string())?;
        Ok(share::invite_code(&Invite { pack_id: s.pack_id.clone(), secret: s.secret.clone(), name: s.name.clone(), addr }))
    }

    pub fn publish(&self, game: &Path, label: &str) -> Result<Option<Changeset>, String> {
        share::publish(game, &self.me(), label)
    }

    /// The owner removes someone (signed with this PC's key) and moves the pack to a new secret.
    pub fn remove(&self, game: &Path, id: &str) -> Result<Removal, String> {
        let secret = hex(&SecretKey::generate().to_bytes());
        share::remove_member(game, self.endpoint.secret_key(), &self.name.lock().unwrap(), id, &secret)
    }

    pub fn set_wishes(&self, game: &Path, items: Vec<serde_json::Value>) -> Result<(), String> {
        share::set_wishes(game, &self.me(), items)
    }

    /// Connect to a member and prove to each other that we both hold this pack's secret. A member who hasn't
    /// heard about a removal yet still has the old secret, so older ones are tried after the current one.
    /// Also returns a newer secret they handed us, if any (adopted only once the owner's removal backs it).
    async fn connect(&self, s: &Share, addr: EndpointAddr) -> Result<(Session, Member, Option<String>), String> {
        let peer = addr.id;
        let me = self.id();
        let secrets = share::secrets(s);
        for (i, secret) in secrets.iter().enumerate() {
            // A failed Hello ends the connection on their side, so each try gets its own.
            let conn = tokio::time::timeout(Duration::from_secs(20), self.endpoint.connect(addr.clone(), ALPN)).await
                .map_err(|_| "Couldn't reach them (are they online with Obelisk open?)".to_string())?
                .map_err(|e| format!("Couldn't connect: {e}"))?;
            let sess = Session { conn };
            let hello = Req::Hello { pack: s.pack_id.clone(), name: self.name.lock().unwrap().clone(), proof: share::proof(secret, &s.pack_id, &me, &peer.to_string()) };
            let reply: HelloReply = match sess.call(&hello).await {
                Ok(v) => serde_json::from_value(v).map_err(|e| e.to_string())?,
                Err(e) if e == WRONG_KEY && i + 1 < secrets.len() => continue,
                Err(e) if e == WRONG_KEY && i > 0 => return Err("This pack's key has changed and they don't have it. Ask its owner for a new invite.".into()),
                Err(e) => return Err(e),
            };
            if let Some(r) = &reply.removed {
                if r.id == me && share::find(&self.instances, &s.pack_id).is_some_and(|g| share::merge_removals(&g, &me, std::slice::from_ref(r)).unwrap_or(false)) {
                    return Err(format!("{} removed you from this pack.", r.by_name));
                }
                return Err("They refused the connection.".into());
            }
            if reply.proof != share::proof(secret, &s.pack_id, &peer.to_string(), &me) {
                return Err("They don't have this pack's key.".into());
            }
            let member = reply.member(peer);
            return Ok((sess, member, reply.rekey));
        }
        Err("They don't have this pack's key.".into())
    }

    /// Pull a friend's changesets and the files they need. Returns how many new changesets arrived.
    pub async fn pull(&self, game: &Path, addr: EndpointAddr) -> Result<usize, String> {
        let s = share::get(game).ok_or("this pack isn't shared")?;
        let (sess, them, rekey) = self.connect(&s, addr).await?;
        let log: LogReply = serde_json::from_value(sess.call(&Req::Log).await?).map_err(|e| e.to_string())?;
        let have: std::collections::HashSet<String> = share::log_ids(game).into_iter().collect();
        let mut got = 0;
        for id in log.ids.into_iter().filter(|i| !have.contains(i)) {
            let cs: Changeset = serde_json::from_value(sess.call(&Req::Changeset { id: id.clone() }).await?).map_err(|e| e.to_string())?;
            if cs.id != id {
                return Err("a friend sent the wrong change".into());
            }
            for h in share::missing_blobs(game, &cs) {
                let bytes = sess.blob(&h).await?;
                share::store_blob(game, &h, &bytes)?;
            }
            share::write_changeset(game, &cs)?;
            got += 1;
        }
        // The owner (for shares joined before owners were passed along), removals, and a new secret if one came with them.
        if let (Some(mut mine), Some(o)) = (share::get(game), log.owner.filter(|o| !o.is_empty())) {
            if mine.owner.is_empty() {
                mine.owner = o;
                share::save(game, &mine)?;
            }
        }
        if share::merge_removals(game, &self.id(), &log.removed)? {
            sess.conn.close(0u32.into(), b"done");
            return Ok(got);
        }
        if let Some(secret) = rekey {
            share::adopt_secret(game, &secret)?;
        }
        let mut seen = log.members;
        seen.push(them);
        share::merge_members(game, &seen)?;
        if let Ok(v) = sess.call(&Req::Wishes).await {
            if let Ok(w) = serde_json::from_value::<HashMap<String, WishList>>(v) {
                share::merge_wishes(game, w)?;
            }
        }
        sess.conn.close(0u32.into(), b"done");
        Ok(got)
    }

    /// Pull from every other member of every shared pack. Quietly skips anyone offline.
    pub async fn sync_all(&self) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        for (inst, game, s) in share::all(&self.instances) {
            let mut total = 0;
            for m in s.members.iter().filter(|m| m.id != self.id()) {
                let Ok(id) = m.id.parse::<EndpointId>() else { continue };
                if let Ok(n) = self.pull(&game, EndpointAddr::from(id)).await {
                    total += n;
                }
            }
            if total > 0 && share::get(&game).is_some_and(|s| s.auto_apply) {
                let _ = share::apply(&game, &[]);
            }
            out.push((inst, total));
        }
        out
    }

    /// Offer a pack under a fresh code for 30 minutes. Returns (code, expires in unix seconds).
    pub async fn offer(&self, game: &Path, manifest: Manifest) -> Result<(String, u64), String> {
        let _ = tokio::time::timeout(Duration::from_secs(8), self.endpoint.online()).await;
        let code = packcode::new_code();
        let expires = packcode::now_secs() + packcode::LIFETIME_SECS;
        let note = packcode::Note { peer: self.id(), expires };
        let c = code.clone();
        tokio::task::spawn_blocking(move || packcode::publish(&c, &note)).await.map_err(|e| e.to_string())??;
        let mut offers = self.offers.lock().unwrap();
        offers.retain(|_, o| o.expires > packcode::now_secs());
        offers.insert(code.clone(), Offer { game: game.to_path_buf(), manifest: Arc::new(manifest), expires, stats: Default::default() });
        drop(offers);
        // The code is the key to its note, so anyone who sees the code could point it at their own PC. While the
        // offer is live, put it back whenever it isn't ours (the receiving side vets everything regardless).
        let (offers, me, c) = (self.offers.clone(), self.id(), code.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(20)).await;
                if !offers.lock().unwrap().get(&c).is_some_and(|o| o.expires > packcode::now_secs()) {
                    break;
                }
                let (c2, me2) = (c.clone(), me.clone());
                let _ = tokio::task::spawn_blocking(move || {
                    if packcode::resolve(&c2).map(|n| n.peer != me2).unwrap_or(true) {
                        let _ = packcode::publish(&c2, &packcode::Note { peer: me2, expires });
                    }
                }).await;
            }
        });
        Ok((code, expires))
    }

    /// Stop offering a code early.
    /// Collaborate → Get a code: the invite to a collaborative pack, handed out under a short code for 30 minutes.
    pub async fn offer_invite(&self, game: &Path, name: &str) -> Result<(String, u64), String> {
        let invite = self.share(game, name).await?;
        let m = Manifest { v: 1, name: name.to_string(), pack: String::new(), files: vec![], icon: None, game: None,
            invite: Some(invite), private: vec![] };
        self.offer(game, m).await
    }

    /// How sharing under `code` is going (None once it's stopped or expired).
    pub fn offer_status(&self, code: &str) -> Option<OfferStatus> {
        let offers = self.offers.lock().unwrap();
        let o = offers.get(code).filter(|o| o.expires > packcode::now_secs())?;
        let s = o.stats.lock().unwrap().clone();
        Some(s)
    }

    pub fn withdraw(&self, code: &str) {
        self.offers.lock().unwrap().remove(code);
    }

    async fn pack_session(&self, code: &str) -> Result<(Session, Manifest), String> {
        let c = code.to_string();
        let note = tokio::task::spawn_blocking(move || packcode::resolve(&c)).await.map_err(|e| e.to_string())??;
        let peer: EndpointId = note.peer.parse().map_err(|_| "The code's note is damaged.")?;
        if peer == self.endpoint.id() {
            return Err("That's your own code. Paste it on the other computer.".into());
        }
        let conn = tokio::time::timeout(Duration::from_secs(30), self.endpoint.connect(EndpointAddr::from(peer), ALPN_PACK)).await
            .map_err(|_| "Couldn't reach the computer sharing it (Obelisk must stay open there until you're done).".to_string())?
            .map_err(|e| format!("Couldn't connect: {e}"))?;
        let sess = Session { conn };
        let m: Manifest = serde_json::from_value(sess.pack_call(&PackReq::Manifest { code: code.into() }).await?)
            .map_err(|_| "The pack's list is damaged.".to_string())?;
        Ok((sess, m))
    }

    /// Look at a pack's code the way the receiver must: every file that would run in the game and doesn't come
    /// straight from Modrinth's or CurseForge's servers is fetched from the sharer and opened. CurseForge files
    /// whose authors block other apps are matched by fingerprint when a CurseForge key is set. Whatever is left
    /// is custom; Windows programs, scripts and code with signs of malware are refused; Defender scans the rest.
    async fn vet(&self, sess: &Session, code: &str, m: &Manifest, cf: Option<Arc<crate::curseforge::Client>>, progress: &Progress) -> Result<Vetting, String> {
        let mut refused = Vec::new();
        let mut code_files = Vec::new();
        let mut known = 0;
        for f in &m.files {
            if packcode::safe_path(&f.path).is_none() {
                let why = if guard::blocked(&f.path) { "a Windows program or script" } else { "outside the pack folders" };
                refused.push(CustomFile { path: f.path.clone(), size: f.size, findings: Findings { serious: vec![why.into()], notes: vec![] } });
            } else if packcode::site_url_ok(&f.url) {
                known += 1;
            } else if guard::risky(&f.path) {
                code_files.push(f.clone());
            }
        }
        // A hostile sharer could list endless files or giant ones to wear this PC out before anything is asked.
        if m.files.len() > 50_000 {
            return Err("This pack lists more files than any real modpack has; not installing it.".into());
        }
        let mut fetched = HashMap::new();
        let mut held = 0usize;
        for (i, f) in code_files.iter().enumerate() {
            progress(ImportProgress { step: "checking", done: i, total: code_files.len(), file: f.path.clone() });
            let bytes = sess.pack_file(code, &f.path).await?;
            held += bytes.len();
            if held > 2 << 30 {
                return Err("This pack's custom code is larger than any real modpack's; not installing it.".into());
            }
            if crate::sha1_hex(&bytes) != f.sha1 {
                return Err(format!("{} didn't match the pack's list", f.path));
            }
            fetched.insert(f.path.clone(), bytes);
        }
        // CurseForge's word for files only it hosts (asked by this PC, not the sharer).
        progress(ImportProgress { step: "checking", done: code_files.len(), total: code_files.len(), file: "Asking CurseForge about its files".into() });
        let genuine: std::collections::HashSet<u32> = match cf {
            Some(cf) => {
                let fps: Vec<u32> = code_files.iter().filter(|f| f.site == "curseforge")
                    .filter_map(|f| fetched.get(&f.path)).map(|b| crate::curseforge::fingerprint(b)).collect();
                tokio::task::spawn_blocking(move || cf.match_fingerprints(&fps)).await.map_err(|e| e.to_string())?
                    .map(|v| v.into_iter().map(|(fp, _, _)| fp).collect()).unwrap_or_default()
            }
            None => Default::default(),
        };
        let mut custom = Vec::new();
        for f in &code_files {
            let bytes = &fetched[&f.path];
            if f.site == "curseforge" && genuine.contains(&crate::curseforge::fingerprint(bytes)) {
                known += 1;
                continue;
            }
            stopped()?;
            let findings = guard::inspect(&f.path, bytes);
            let c = CustomFile { path: f.path.clone(), size: f.size, findings };
            if c.findings.serious.is_empty() { custom.push(c) } else { refused.push(c) }
        }
        // Defender looks at the custom files before anything is installed.
        let scan = if custom.is_empty() { Scan::Clean } else {
            progress(ImportProgress { step: "scanning", done: 0, total: custom.len(), file: "Windows Defender is scanning the custom files".into() });
            let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
            for c in &custom {
                let to = dir.path().join(&c.path);
                if let Some(d) = to.parent() { std::fs::create_dir_all(d).map_err(|e| e.to_string())? }
                std::fs::write(&to, &fetched[&c.path]).map_err(|e| e.to_string())?;
            }
            let p = dir.path().to_path_buf();
            let s = scan_watched(p, custom.len(), "custom files", progress).await?;
            stopped()?;
            s
        };
        Ok(Vetting { known, custom, refused, scan, fetched })
    }

    /// What a code's pack holds (before choosing about custom files).
    pub async fn preview(&self, code: &str, cf: Option<Arc<crate::curseforge::Client>>,
                         progress: impl Fn(ImportProgress) + Send + Sync + 'static) -> Result<PackPreview, String> {
        STOP.store(false, std::sync::atomic::Ordering::SeqCst);
        let (sess, m) = self.pack_session(code).await?;
        if let Some(inv) = m.invite.clone() {
            sess.conn.close(0u32.into(), b"invite");
            share::parse_invite(&inv)?;
            return Ok(PackPreview { name: m.name.clone(), files: 0, bytes: 0, known: 0, custom: vec![], refused: vec![],
                scan: Scan::Clean, game: None, invite: Some(inv) });
        }
        let v = self.vet(&sess, code, &m, cf, &progress).await;
        // Only a finished import hangs up with "done" (the sharer's screen then says they can close).
        sess.conn.close(0u32.into(), b"checked");
        let v = v?;
        Ok(PackPreview { name: m.name.clone(), files: m.files.len(), bytes: m.files.iter().map(|f| f.size).sum(),
            known: v.known, custom: v.custom, refused: v.refused, scan: v.scan, game: packcode::vet_game_settings(m.game.clone()), invite: None })
    }

    /// Install a code's pack as a new profile. Mods come from their sites where possible (checked against the
    /// list), everything else from the sharer, vetted again here (the sharer could change the pack after the
    /// preview). Custom code only with `with_custom`; refused files never. Defender scans the finished profile,
    /// and a threat removes it again.
    pub async fn import(&self, code: &str, with_custom: bool, cf: Option<Arc<crate::curseforge::Client>>,
                        progress: impl Fn(ImportProgress) + Send + Sync + 'static) -> Result<ImportReport, String> {
        STOP.store(false, std::sync::atomic::Ordering::SeqCst);
        let (sess, m) = self.pack_session(code).await?;
        if let Some(inv) = m.invite {
            // A Collaborate code: join the pack instead (blocked files are listed as left out).
            sess.conn.close(0u32.into(), b"invite");
            let id = self.join(&inv).await?;
            let held = crate::held::list(&self.instances.join(&id).join("minecraft"));
            let left_out = held.iter().map(|h| format!("{} (blocked: {})", h.path, h.reasons.join(", "))).collect();
            return Ok(ImportReport { id, left_out, scan: Scan::Clean });
        }
        let vet = match self.vet(&sess, code, &m, cf, &progress).await {
            Ok(v) => v,
            Err(e) => { sess.conn.close(0u32.into(), b"failed"); return Err(e) }
        };
        if let Scan::Threat(t) = &vet.scan {
            sess.conn.close(0u32.into(), b"done");
            return Err(format!("Windows Defender found a threat in this pack's files ({t}). Nothing was installed."));
        }
        // Code with signs of malware was opened here (it's in fetched): kept aside for the user to decide (held.rs).
        // Windows programs and files outside the pack never come in.
        let mut left_out: Vec<String> = vet.refused.iter().map(|c| format!("{} ({}: {})", c.path,
            if vet.fetched.contains_key(&c.path) { "blocked" } else { "refused" }, c.findings.serious.join(", "))).collect();
        let skip: std::collections::HashSet<String> = vet.refused.iter().map(|c| c.path.clone())
            .chain(vet.custom.iter().filter(|_| !with_custom).map(|c| c.path.clone())).collect();
        if !with_custom {
            left_out.extend(vet.custom.iter().map(|c| format!("{} (custom code you chose to leave out)", c.path)));
        }
        let files: Vec<PackFile> = m.files.iter().filter(|f| !skip.contains(&f.path)).cloned().collect();
        let fetched = vet.fetched;
        let left_out = Arc::new(Mutex::new(left_out));
        let (id, game) = packcode::create_instance(&self.instances, &m)?;
        for c in &vet.refused {
            if let Some(bytes) = fetched.get(&c.path) {
                if let Err(e) = crate::held::put(&game, &c.path, bytes, "", c.findings.serious.clone()) {
                    share::discard(&self.instances.join(&id));
                    return Err(e);
                }
            }
        }
        let total = files.len();
        let progress = Arc::new(progress);
        let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        // Files the sharer sent (not checked against Modrinth's or CurseForge's own copy): the ones Defender scans after.
        let mut sent: Vec<(String, Vec<u8>)> = Vec::new();
        let result: Result<(), String> = async {
            // Site downloads, a few at a time; anything that fails comes from the sharer instead.
            let queue = Arc::new(Mutex::new(files.iter().filter(|f| packcode::site_url_ok(&f.url)).cloned().collect::<Vec<_>>()));
            let failed = Arc::new(Mutex::new(Vec::<PackFile>::new()));
            let mut workers = Vec::new();
            for _ in 0..6 {
                let (queue, failed, done, game, progress) = (queue.clone(), failed.clone(), done.clone(), game.clone(), progress.clone());
                workers.push(tokio::task::spawn_blocking(move || loop {
                    if stopped().is_err() { break }
                    let Some(f) = queue.lock().unwrap().pop() else { break };
                    match packcode::from_site(&f).map(|b| packcode::place(&game, &f, &b)) {
                        Some(Ok(())) => {
                            let n = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                            progress(ImportProgress { step: "installing", done: n, total, file: f.path.clone() });
                        }
                        _ => failed.lock().unwrap().push(f),
                    }
                }));
            }
            for w in workers {
                w.await.map_err(|e| e.to_string())?;
            }
            stopped()?;
            let mut rest: Vec<PackFile> = files.iter().filter(|f| !packcode::site_url_ok(&f.url)).cloned().collect();
            let failed: Vec<PackFile> = failed.lock().unwrap().drain(..).collect();
            for f in failed {
                // The site didn't give the listed file. Code then counts as custom and is vetted like any other.
                if !guard::risky(&f.path) {
                    rest.push(f);
                    continue;
                }
                if !with_custom {
                    left_out.lock().unwrap().push(format!("{} (its site didn't have the listed file)", f.path));
                    continue;
                }
                let bytes = sess.pack_file(code, &f.path).await?;
                let findings = guard::inspect(&f.path, &bytes);
                if !findings.serious.is_empty() {
                    crate::held::put(&game, &f.path, &bytes, "", findings.serious.clone())?;
                    left_out.lock().unwrap().push(format!("{} (blocked: {})", f.path, findings.serious.join(", ")));
                    continue;
                }
                packcode::place(&game, &f, &bytes)?;
                sent.push((f.path.clone(), bytes));
                let n = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                progress(ImportProgress { step: "installing", done: n, total, file: f.path.clone() });
            }
            for f in rest {
                let bytes = match fetched.get(&f.path) {
                    Some(b) => b.clone(),
                    None => sess.pack_file(code, &f.path).await?,
                };
                if guard::mod_record(&f.path) && !guard::mod_record_ok(&bytes) {
                    left_out.lock().unwrap().push(format!("{} (its mod's update link isn't Modrinth or CurseForge)", f.path));
                    let n = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    progress(ImportProgress { step: "installing", done: n, total, file: f.path.clone() });
                    continue;
                }
                packcode::place(&game, &f, &bytes)?;
                sent.push((f.path.clone(), bytes));
                let n = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                progress(ImportProgress { step: "installing", done: n, total, file: f.path.clone() });
            }
            Ok(())
        }.await;
        sess.conn.close(0u32.into(), b"done");
        if let Err(e) = result {
            // Don't leave a half-made profile behind.
            share::discard(&self.instances.join(&id));
            return Err(e);
        }
        // Defender scans everything the sharer sent, exactly as installed. Files from Modrinth and CurseForge are
        // byte-for-byte the sites' own copies (their hashes were checked), and real-time protection saw each as it was written.
        let scan = if sent.is_empty() { Scan::Clean } else {
            let dir = tempfile::tempdir().map_err(|e| e.to_string());
            let scanned = match dir {
                Ok(dir) => {
                    let mut r = Ok(());
                    for (path, bytes) in &sent {
                        let to = dir.path().join(path);
                        if let Some(d) = to.parent() { if let Err(e) = std::fs::create_dir_all(d) { r = Err(e.to_string()); break } }
                        if let Err(e) = std::fs::write(&to, bytes) { r = Err(e.to_string()); break }
                    }
                    match r {
                        Ok(()) => scan_watched(dir.path().to_path_buf(), sent.len(), "files your friend sent", &*progress).await.and_then(|s| stopped().map(|_| s)),
                        Err(e) => Err(e),
                    }
                }
                Err(e) => Err(e),
            };
            match scanned {
                Ok(s) => s,
                Err(e) => {
                    share::discard(&self.instances.join(&id));
                    return Err(e);
                }
            }
        };
        if let Scan::Threat(t) = &scan {
            share::discard(&self.instances.join(&id));
            return Err(format!("Windows Defender found a threat in the installed pack ({t}). It has been removed."));
        }
        let left_out = left_out.lock().unwrap().clone();
        Ok(ImportReport { id, left_out, scan })
    }

    /// The invite a short Collaborate code carries.
    async fn invite_of(&self, code: &str) -> Result<String, String> {
        let (sess, m) = self.pack_session(code).await?;
        sess.conn.close(0u32.into(), b"invite");
        m.invite.ok_or_else(|| "That's a pack code, not a Collaborate code: paste it into Import to install the pack.".to_string())
    }

    /// Join a friend's pack from an invite code (the long one, or the short code it's offered under): creates the profile, downloads the pack, applies it.
    /// Returns the new profile's folder name.
    pub async fn join(&self, code: &str) -> Result<String, String> {
        let long;
        let code = if code.trim().starts_with("obelisk1.") { code } else {
            long = self.invite_of(&packcode::normalize(code).map_err(|_| "That isn't a Collaborate code. Codes are 6 letters and digits, like K7MQ2X.".to_string())?).await?;
            &long
        };
        let inv = share::parse_invite(code)?;
        if let Some(game) = share::find(&self.instances, &inv.pack_id) {
            return Err(format!("You already have this pack ({}).", game.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
        }
        let addr: EndpointAddr = serde_json::from_value(inv.addr.clone()).map_err(|_| "The invite code is damaged.")?;
        let host = addr.id;
        let s = Share { pack_id: inv.pack_id.clone(), name: inv.name.clone(), secret: inv.secret.clone(),
            members: vec![self.me(), Member { id: host.to_string(), name: String::new(), last_seen_ms: now_ms() }], auto_apply: false, created_ms: now_ms(),
            owner: String::new(), removed: vec![], epoch: 0, old_secrets: vec![] };
        let (sess, _, _) = self.connect(&s, addr.clone()).await?;
        let files: InstanceReply = serde_json::from_value(sess.call(&Req::Instance).await?).map_err(|e| e.to_string())?;
        sess.conn.close(0u32.into(), b"done");
        let (id, game) = share::create_instance(&self.instances, &s, &files.cfg, &files.pack)?;
        if let Some(key) = files.icon.as_deref().and_then(|i| packcode::take_icon(&crate::icons_dir(&crate::data_dir()), &id, i)) {
            let _ = crate::update_cfg(&self.instances.join(&id).join("instance.cfg"), &[("iconKey", Some(key))]);
        }
        let result = async {
            self.pull(&game, addr).await?;
            share::apply(&game, &[])
        }.await;
        if let Err(e) = result {
            // Don't leave a half-made profile behind.
            share::discard(&self.instances.join(&id));
            return Err(e);
        }
        Ok(id)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelloReply {
    name: String,
    proof: String,
    /// They were removed from the pack: here's the owner's signed note (no access follows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    removed: Option<Removal>,
    /// They used an older secret: the current one (they check it against the owner's removal first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rekey: Option<String>,
}

impl HelloReply {
    fn member(&self, id: EndpointId) -> Member {
        Member { id: id.to_string(), name: self.name.clone(), last_seen_ms: now_ms() }
    }
}

#[derive(Serialize, Deserialize)]
struct LogReply {
    ids: Vec<String>,
    members: Vec<Member>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    removed: Vec<Removal>,
}

#[derive(Serialize, Deserialize)]
struct InstanceReply {
    cfg: String,
    pack: String,
    /// The pack's picture (base64), if it has its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
}

struct Session {
    conn: Connection,
}

impl Session {
    async fn raw(&self, req: &Req, limit: usize) -> Result<(Resp, Vec<u8>), String> {
        stopped()?;
        let (mut send, mut recv) = self.conn.open_bi().await.map_err(|e| e.to_string())?;
        send.write_all(&serde_json::to_vec(req).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())?;
        send.finish().map_err(|e| e.to_string())?;
        // Read as it comes: a big file may take a while, but a sharer that sends nothing for STALL has stopped.
        let mut all = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            stopped()?;
            let n = tokio::time::timeout(STALL, recv.read(&mut buf)).await
                .map_err(|_| format!("The sharing PC stopped sending (nothing for {} seconds). It may have gone to sleep or closed Obelisk.", STALL.as_secs()))?
                .map_err(|e| e.to_string())?;
            let Some(n) = n else { break };
            all.extend_from_slice(&buf[..n]);
            if all.len() > limit {
                return Err("a reply was larger than allowed".into());
            }
        }
        if all.len() < 4 {
            return Err("empty reply".into());
        }
        let n = u32::from_be_bytes([all[0], all[1], all[2], all[3]]) as usize;
        let head: Resp = serde_json::from_slice(all.get(4..4 + n).ok_or("bad reply")?).map_err(|e| e.to_string())?;
        if let Some(e) = head.error.clone() {
            return Err(e);
        }
        Ok((head, all[4 + n..].to_vec()))
    }

    async fn call(&self, req: &Req) -> Result<serde_json::Value, String> {
        Ok(self.raw(req, MAX_JSON).await?.0.value.unwrap_or_default())
    }

    async fn pack_call(&self, req: &PackReq) -> Result<serde_json::Value, String> {
        Ok(self.pack_raw(req, MAX_JSON).await?.0.value.unwrap_or_default())
    }

    async fn pack_file(&self, code: &str, path: &str) -> Result<Vec<u8>, String> {
        let (head, body) = self.pack_raw(&PackReq::File { code: code.into(), path: path.into() }, MAX_BLOB + 4096).await?;
        if body.len() as u64 != head.size {
            return Err("a file arrived incomplete".into());
        }
        Ok(body)
    }

    async fn pack_raw(&self, req: &PackReq, limit: usize) -> Result<(Resp, Vec<u8>), String> {
        let (mut send, mut recv) = self.conn.open_bi().await.map_err(|e| e.to_string())?;
        send.write_all(&serde_json::to_vec(req).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())?;
        send.finish().map_err(|e| e.to_string())?;
        let all = tokio::time::timeout(Duration::from_secs(600), recv.read_to_end(limit)).await
            .map_err(|_| "timed out".to_string())?.map_err(|e| e.to_string())?;
        if all.len() < 4 {
            return Err("empty reply".into());
        }
        let n = u32::from_be_bytes([all[0], all[1], all[2], all[3]]) as usize;
        let head: Resp = serde_json::from_slice(all.get(4..4 + n).ok_or("bad reply")?).map_err(|e| e.to_string())?;
        if let Some(e) = head.error.clone() {
            return Err(e);
        }
        Ok((head, all[4 + n..].to_vec()))
    }

    async fn blob(&self, hash: &str) -> Result<Vec<u8>, String> {
        let (head, body) = self.raw(&Req::Blob { hash: hash.into() }, MAX_BLOB + 4096).await?;
        if body.len() as u64 != head.size {
            return Err("a file arrived incomplete".into());
        }
        Ok(body)
    }
}

#[derive(Debug, Clone)]
struct Handler {
    instances: PathBuf,
    name: Arc<Mutex<String>>,
    me: EndpointId,
}

impl ProtocolHandler for Handler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let peer = conn.remote_id();
        let mut authed: Option<PathBuf> = None;
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            let Ok(bytes) = recv.read_to_end(64 * 1024).await else { break };
            let (head, body) = match serde_json::from_slice::<Req>(&bytes) {
                Ok(req) => self.answer(req, peer, &mut authed),
                Err(_) => (Resp { error: Some("bad request".into()), ..Default::default() }, Vec::new()),
            };
            let h = serde_json::to_vec(&head).unwrap_or_default();
            let mut out = (h.len() as u32).to_be_bytes().to_vec();
            out.extend(h);
            if send.write_all(&out).await.is_err() || send.write_all(&body).await.is_err() {
                break;
            }
            let _ = send.finish();
            if authed.is_none() {
                break; // failed Hello: nothing more for them
            }
        }
        conn.closed().await;
        Ok(())
    }
}

impl Handler {
    fn answer(&self, req: Req, peer: EndpointId, authed: &mut Option<PathBuf>) -> (Resp, Vec<u8>) {
        let ok = |v: serde_json::Value| Resp { value: Some(v), ..Default::default() };
        let err = |e: &str| Resp { error: Some(e.into()), ..Default::default() };
        if let Req::Hello { pack, name, proof } = &req {
            let Some(game) = share::find(&self.instances, pack) else { return (err("That pack isn't shared from here any more."), vec![]) };
            let Some(s) = share::get(&game) else { return (err("not shared"), vec![]) };
            let (peer_id, me) = (peer.to_string(), self.me.to_string());
            let Some((i, secret)) = share::secrets(&s).into_iter().enumerate().find(|(_, k)| *proof == share::proof(k, pack, &peer_id, &me)) else {
                return (err(WRONG_KEY), vec![]);
            };
            let mine = HelloReply { name: self.name.lock().unwrap().clone(), proof: String::new(), removed: None, rekey: None };
            if let Some(r) = s.removed.iter().find(|r| r.id == peer_id) {
                return (ok(serde_json::to_value(HelloReply { removed: Some(r.clone()), ..mine }).unwrap_or_default()), vec![]);
            }
            // An older secret works only for people already in the pack (it may be in a removed person's hands);
            // they get the current one, which they accept only if the owner's signed removal names it.
            let stale = i > 0;
            if stale && !s.members.iter().any(|m| m.id == peer_id) {
                return (err("This invite is no longer valid. Ask for a new one."), vec![]);
            }
            let _ = share::merge_members(&game, &[Member { id: peer_id.clone(), name: name.clone(), last_seen_ms: now_ms() }]);
            *authed = Some(game);
            let reply = HelloReply { proof: share::proof(&secret, pack, &me, &peer_id), rekey: stale.then(|| s.secret.clone()), ..mine };
            return (ok(serde_json::to_value(reply).unwrap_or_default()), vec![]);
        }
        let Some(game) = authed.as_ref() else { return (err("say hello first"), vec![]) };
        match req {
            Req::Log => {
                let Some(s) = share::get(game) else { return (err("not shared"), vec![]) };
                let owner = Some(share::owner(game, &s));
                (ok(serde_json::to_value(LogReply { ids: share::log_ids(game), members: s.members, owner, removed: s.removed }).unwrap_or_default()), vec![])
            }
            Req::Wishes => (ok(serde_json::to_value(share::wishes(game)).unwrap_or_default()), vec![]),
            Req::Changeset { id } => match share::read_changeset(game, &id) {
                Some(cs) => (ok(serde_json::to_value(cs).unwrap_or_default()), vec![]),
                None => (err("no such change"), vec![]),
            },
            Req::Blob { hash } => {
                // Only files the log refers to, never arbitrary store contents.
                let listed = share::referenced(game).contains(&hash);
                match share::blob_path(game, &hash).filter(|_| listed).and_then(|p| std::fs::read(p).ok()) {
                    Some(bytes) => (Resp { size: bytes.len() as u64, ..Default::default() }, bytes),
                    None => (err("missing file"), vec![]),
                }
            }
            Req::Instance => {
                let name = share::get(game).map(|s| s.name).unwrap_or_default();
                match share::instance_files(game, &name) {
                    Ok((cfg, pack)) => {
                        let icon = game.parent().and_then(packcode::pack_icon);
                        (ok(serde_json::to_value(InstanceReply { cfg, pack, icon }).unwrap_or_default()), vec![])
                    }
                    Err(e) => (err(&e), vec![]),
                }
            }
            Req::Hello { .. } => unreachable!(),
        }
    }
}

/// Answers pack-code requests: the list, and files on it, for a live code only.
#[derive(Debug, Clone)]
struct PackHandler {
    offers: Offers,
}

impl ProtocolHandler for PackHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        // The code's counters, once this friend has asked for something under a live code.
        let mut stats: Option<Arc<Mutex<OfferStatus>>> = None;
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            let Ok(bytes) = recv.read_to_end(64 * 1024).await else { break };
            let (head, body, ok) = match serde_json::from_slice::<PackReq>(&bytes) {
                Ok(req) => {
                    let (code, file) = match &req { PackReq::Manifest { code } => (code.clone(), false), PackReq::File { code, .. } => (code.clone(), true) };
                    let r = self.answer(req);
                    if r.2 {
                        if stats.is_none() {
                            stats = self.offers.lock().unwrap().get(&code).map(|o| o.stats.clone());
                            if let Some(s) = &stats { s.lock().unwrap().connected += 1 }
                        }
                        if let Some(s) = &stats {
                            let mut s = s.lock().unwrap();
                            if file { s.sent += 1 }
                            s.last_ms = packcode::now_secs() * 1000;
                        }
                    }
                    r
                }
                Err(_) => (Resp { error: Some("bad request".into()), ..Default::default() }, Vec::new(), false),
            };
            let h = serde_json::to_vec(&head).unwrap_or_default();
            let mut out = (h.len() as u32).to_be_bytes().to_vec();
            out.extend(h);
            if send.write_all(&out).await.is_err() || send.write_all(&body).await.is_err() {
                break;
            }
            let _ = send.finish();
            if !ok {
                break; // a wrong or dead code gets nothing more on this connection
            }
        }
        let why = conn.closed().await;
        if let Some(s) = stats {
            let mut s = s.lock().unwrap();
            s.connected = s.connected.saturating_sub(1);
            // An import that went all the way through says so as it hangs up.
            if matches!(&why, iroh::endpoint::ConnectionError::ApplicationClosed(c) if &c.reason[..] == b"done") {
                s.finished += 1;
            }
        }
        Ok(())
    }
}

impl PackHandler {
    fn answer(&self, req: PackReq) -> (Resp, Vec<u8>, bool) {
        let err = |e: &str| (Resp { error: Some(e.into()), ..Default::default() }, Vec::new(), false);
        let code = match &req { PackReq::Manifest { code } | PackReq::File { code, .. } => code.clone() };
        let offer = self.offers.lock().unwrap().get(&code).cloned();
        let Some(offer) = offer.filter(|o| o.expires > packcode::now_secs()) else { return err("That code has expired. Ask for a new one.") };
        match req {
            PackReq::Manifest { .. } => (Resp { value: serde_json::to_value(&*offer.manifest).ok(), ..Default::default() }, Vec::new(), true),
            PackReq::File { path, .. } => {
                // Only files on the offered list, and only while they're still what the list says.
                let Some(f) = offer.manifest.files.iter().find(|f| f.path == path) else { return err("not in this pack") };
                let Some(rel) = packcode::safe_path(&f.path) else { return err("not in this pack") };
                match std::fs::read(offer.game.join(rel)) {
                    Ok(b) if crate::sha1_hex(&b) == f.sha1 => (Resp { size: b.len() as u64, ..Default::default() }, b, true),
                    Ok(_) => err(&format!("{path} changed since the code was made; ask for a new code")),
                    Err(_) => err(&format!("{path} is gone from the sharer's pack; ask for a new code")),
                }
            }
        }
    }
}

/// A fresh identity key for this computer's peer.
pub fn new_key() -> [u8; 32] {
    SecretKey::generate().to_bytes()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two Obelisks on this PC, over the real relay and n0's network: run by hand with
    /// `cargo test --lib -- --ignored pack_code`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn pack_code_end_to_end() {
        let t = tempfile::tempdir().unwrap();
        let (a_inst, b_inst) = (t.path().join("a"), t.path().join("b"));
        let game = a_inst.join("Pack/minecraft");
        let evil = "x discord.com/api/webhooks/1/2 x";
        for (p, c) in [("mods/mine.jar", "custom jar"), ("config/a.toml", "x=1"), ("mods/evil.jar", evil), ("mods/setup.exe", "MZ")] {
            std::fs::create_dir_all(game.join(p).parent().unwrap()).unwrap();
            std::fs::write(game.join(p), c).unwrap();
        }
        std::fs::write(a_inst.join("Pack/mmc-pack.json"), "{\"components\":[]}").unwrap();
        std::fs::create_dir_all(&b_inst).unwrap();
        let a = Net::start(new_key(), a_inst.clone(), "A".into()).await.unwrap();
        let b = Net::start(new_key(), b_inst.clone(), "B".into()).await.unwrap();
        let mut m = packcode::manifest(&game, "Pack", None, None).unwrap();
        // A lying list: the stealer claims to be a Modrinth file (the link is dead, so it'd come from the sharer).
        let e = m.files.iter_mut().find(|f| f.path == "mods/evil.jar").unwrap();
        (e.site, e.url, e.custom) = ("modrinth".into(), "https://cdn.modrinth.com/data/AAAAAAAA/versions/x/evil.jar".into(), false);
        let (code, _) = a.offer(&game, m).await.unwrap();

        let p = b.preview(&code, None, |_| {}).await.unwrap();
        assert_eq!(p.custom.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), ["mods/mine.jar"]);
        assert!(p.refused.iter().any(|c| c.path == "mods/setup.exe"), "{p:?}");
        let r = b.import(&code, false, None, |_| {}).await.unwrap();
        let id = r.id;
        let got = b_inst.join(&id).join("minecraft");
        assert_eq!(std::fs::read_to_string(got.join("config/a.toml")).unwrap(), "x=1");
        assert!(!got.join("mods/mine.jar").exists(), "custom files only when chosen");
        assert!(!got.join("mods/evil.jar").exists() && !got.join("mods/setup.exe").exists());
        let r2 = b.import(&code, true, None, |_| {}).await.unwrap();
        let got2 = b_inst.join(&r2.id).join("minecraft");
        assert_eq!(std::fs::read_to_string(got2.join("mods/mine.jar")).unwrap(), "custom jar");
        assert!(!got2.join("mods/evil.jar").exists(), "a lying list doesn't sneak code past the checks: {:?}", r2.left_out);
        assert!(r2.left_out.iter().any(|l| l.contains("evil.jar") && l.contains("webhook")), "{:?}", r2.left_out);

        a.withdraw(&code);
        assert!(b.preview(&code, None, |_| {}).await.is_err(), "a withdrawn code serves nothing");
        assert!(a.preview(&code, None, |_| {}).await.unwrap_err().contains("your own code"));
    }

    /// Owner, member and a member who gets removed, over the real network: run by hand with
    /// `cargo test --lib -- --ignored collab_removal`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn collab_removal_end_to_end() {
        let t = tempfile::tempdir().unwrap();
        let inst = |n: &str| t.path().join(n);
        let game = inst("o").join("Pack/minecraft");
        std::fs::create_dir_all(game.join("config")).unwrap();
        std::fs::write(game.join("config/a.toml"), "x=1").unwrap();
        std::fs::write(inst("o").join("Pack/mmc-pack.json"), "{\"components\":[]}").unwrap();
        std::fs::write(inst("o").join("Pack/instance.cfg"), "InstanceType=OneSix
name=Pack
").unwrap();
        for n in ["m", "x"] { std::fs::create_dir_all(inst(n)).unwrap(); }
        let o = Net::start(new_key(), inst("o"), "Olive".into()).await.unwrap();
        let m = Net::start(new_key(), inst("m"), "Mia".into()).await.unwrap();
        let x = Net::start(new_key(), inst("x"), "Xan".into()).await.unwrap();
        let code = o.share(&game, "Pack").await.unwrap();
        let gm = inst("m").join(m.join(&code).await.unwrap()).join("minecraft");
        let gx = inst("x").join(x.join(&code).await.unwrap()).join("minecraft");
        assert_eq!(share::get(&gm).unwrap().owner, o.id(), "the owner comes along when joining");

        x.set_wishes(&gx, vec![serde_json::json!({ "hit": { "project_id": "AANobbMI" } })]).unwrap();
        o.sync_all().await;
        assert_eq!(share::wishes(&game).get(&x.id()).map(|w| w.name.as_str()), Some("Xan"), "wish lists travel");

        o.remove(&game, &x.id()).unwrap();
        assert!(m.remove(&gm, &o.id()).is_err(), "only the owner removes people");
        m.sync_all().await;
        let sm = share::get(&gm).unwrap();
        assert_eq!(sm.secret, share::get(&game).unwrap().secret, "the member moved to the new secret");
        assert!(!sm.members.iter().any(|mm| mm.id == x.id()));
        x.sync_all().await;
        assert!(share::get(&gx).is_none(), "the removed one stops collaborating");
        assert_eq!(share::ended(&gx).unwrap().by_name, "Olive");
        // Their old invite no longer lets anyone in under a new id.
        let z = Net::start(new_key(), inst("z"), "Zed".into()).await.unwrap();
        std::fs::create_dir_all(inst("z")).unwrap();
        assert!(z.join(&code).await.is_err());
    }

    /// A friend joins through Collaborate's short code (in Join and in Import), a jar that reads saved accounts is
    /// held for the user instead of dropped, the pack's picture comes along, and a half-made folder from an
    /// earlier failed join doesn't block joining. Needs the network: `cargo test --lib -- --ignored collab_short`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn collab_short_code_end_to_end() {
        let t = tempfile::tempdir().unwrap();
        // Prism's data folder (icons) for both sides of this test.
        // SAFETY: set before any thread of this test reads it; run on its own (see above).
        unsafe { std::env::set_var("OBELISK_PRISM_DIR", t.path().join("prism")) };
        let png: &[u8] = b"\x89PNG\r\n\x1a\n0000IHDR fake picture";
        std::fs::create_dir_all(t.path().join("prism/icons")).unwrap();
        std::fs::write(t.path().join("prism/icons/obelisk-pack-1234.png"), png).unwrap();
        let inst = |n: &str| t.path().join(n);
        let game = inst("o").join("Pack/minecraft");
        std::fs::create_dir_all(game.join("config")).unwrap();
        std::fs::create_dir_all(game.join("mods")).unwrap();
        std::fs::write(game.join("config/a.toml"), "x=1").unwrap();
        let mut jar = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        jar.start_file("gg/essential/Accounts.class", zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut jar, b"\xca\xfe\xba\xbe .minecraft/microsoft_accounts.json").unwrap();
        std::fs::write(game.join("mods/Essential.jar"), jar.finish().unwrap().into_inner()).unwrap();
        std::fs::write(inst("o").join("Pack/mmc-pack.json"), "{\"components\":[]}").unwrap();
        std::fs::write(inst("o").join("Pack/instance.cfg"), "InstanceType=OneSix\nname=Pack\niconKey=obelisk-pack-1234\n").unwrap();
        for n in ["m", "x"] { std::fs::create_dir_all(inst(n)).unwrap(); }
        let o = Net::start(new_key(), inst("o"), "Olive".into()).await.unwrap();
        let m = Net::start(new_key(), inst("m"), "Mia".into()).await.unwrap();
        let x = Net::start(new_key(), inst("x"), "Xan".into()).await.unwrap();
        let (short, _) = o.offer_invite(&game, "Pack").await.unwrap();
        let pack_id = share::get(&game).unwrap().pack_id;

        // Join with the short code; an earlier failed join left a folder that isn't a profile.
        let junk = inst("m").join("Pack/minecraft/.obelisk/share");
        std::fs::create_dir_all(&junk).unwrap();
        std::fs::write(junk.join("share.json"), serde_json::to_vec(&serde_json::json!({ "packId": pack_id, "name": "Pack", "secret": "x",
            "members": [], "autoApply": false, "createdMs": 0 })).unwrap()).unwrap();
        assert!(share::find(&inst("m"), &pack_id).is_none(), "a half-made folder isn't a pack you have");
        assert_eq!(share::half_made(&inst("m")), vec![inst("m").join("Pack")]);
        let id = m.join(&short.to_lowercase()).await.unwrap();
        let gm = inst("m").join(&id).join("minecraft");
        assert_eq!(std::fs::read_to_string(gm.join("config/a.toml")).unwrap(), "x=1");
        assert!(!gm.join("mods/Essential.jar").exists());
        let held = crate::held::list(&gm);
        assert_eq!(held.len(), 1, "{held:?}");
        assert_eq!((held[0].path.as_str(), held[0].from.as_str()), ("mods/Essential.jar", "Olive"));
        let cfg = crate::read_cfg(&inst("m").join(&id).join("instance.cfg"));
        assert!(crate::icon_data_url(&t.path().join("prism"), &cfg["iconKey"]).is_some(), "the pack's picture came along: {cfg:?}");
        assert!(share::local_changes(&gm).unwrap().is_empty(), "the blocked jar isn't sent back as deleted");
        assert!(m.join(&short).await.unwrap_err().contains("already have"));
        crate::held::allow(&gm, "mods/Essential.jar").unwrap();
        assert!(gm.join("mods/Essential.jar").is_file() && share::local_changes(&gm).unwrap().is_empty());

        // The same short code pasted into Import joins too.
        let r = x.import(&short, false, None, |_| {}).await.unwrap();
        let gx = inst("x").join(&r.id).join("minecraft");
        assert!(share::get(&gx).is_some(), "Import joined the collaborative pack");
        assert!(r.left_out.iter().any(|l| l.starts_with("mods/Essential.jar (blocked")), "{:?}", r.left_out);
        assert_eq!(crate::held::list(&gx).len(), 1);
    }
}
