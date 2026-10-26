//! The network side of shared packs: Obelisk talking to Obelisk directly over iroh (encrypted QUIC,
//! dialed by public key, through home routers; n0's public relays are used only when a direct
//! path can't be made, and they can't read the traffic).
//!
//! Each request is one bi-directional stream: a JSON request, then a JSON header (plus raw file
//! bytes for blobs). The first request on a connection must be Hello, where both sides prove they
//! know the pack's secret without sending it. Nothing is served before that.

use crate::guard::{self, Findings, Scan};
use crate::packcode::{self, Manifest, PackFile};
use crate::share::{self, Changeset, Invite, Member, Share};
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
const MAX_BLOB: usize = 600 << 20;

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "t", rename_all = "camelCase")]
enum Req {
    Hello { pack: String, name: String, proof: String },
    Log,
    Changeset { id: String },
    Blob { hash: String },
    Instance,
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

/// Progress of an incoming pack: files done of total, and the one just finished.
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ImportProgress {
    pub done: usize,
    pub total: usize,
    pub file: String,
}

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

    /// Connect to a member and prove to each other that we both hold this pack's secret.
    async fn connect(&self, s: &Share, addr: EndpointAddr) -> Result<(Session, Member), String> {
        let peer = addr.id;
        let conn = tokio::time::timeout(Duration::from_secs(20), self.endpoint.connect(addr, ALPN)).await
            .map_err(|_| "Couldn't reach them (are they online with Obelisk open?)".to_string())?
            .map_err(|e| format!("Couldn't connect: {e}"))?;
        let sess = Session { conn };
        let me = self.id();
        let hello = Req::Hello { pack: s.pack_id.clone(), name: self.name.lock().unwrap().clone(), proof: share::proof(&s.secret, &s.pack_id, &me, &peer.to_string()) };
        let reply: HelloReply = serde_json::from_value(sess.call(&hello).await?).map_err(|e| e.to_string())?;
        if reply.proof != share::proof(&s.secret, &s.pack_id, &peer.to_string(), &me) {
            return Err("They don't have this pack's key.".into());
        }
        Ok((sess, reply.member(peer)))
    }

    /// Pull a friend's changesets and the files they need. Returns how many new changesets arrived.
    pub async fn pull(&self, game: &Path, addr: EndpointAddr) -> Result<usize, String> {
        let s = share::get(game).ok_or("this pack isn't shared")?;
        let (sess, them) = self.connect(&s, addr).await?;
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
        let mut seen = log.members;
        seen.push(them);
        share::merge_members(game, &seen)?;
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
        offers.insert(code.clone(), Offer { game: game.to_path_buf(), manifest: Arc::new(manifest), expires });
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
    async fn vet(&self, sess: &Session, code: &str, m: &Manifest, cf: Option<Arc<crate::curseforge::Client>>) -> Result<Vetting, String> {
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
        for f in &code_files {
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
            let findings = guard::inspect(&f.path, bytes);
            let c = CustomFile { path: f.path.clone(), size: f.size, findings };
            if c.findings.serious.is_empty() { custom.push(c) } else { refused.push(c) }
        }
        // Defender looks at the custom files before anything is installed.
        let scan = if custom.is_empty() { Scan::Clean } else {
            let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
            for c in &custom {
                let to = dir.path().join(&c.path);
                if let Some(d) = to.parent() { std::fs::create_dir_all(d).map_err(|e| e.to_string())? }
                std::fs::write(&to, &fetched[&c.path]).map_err(|e| e.to_string())?;
            }
            let p = dir.path().to_path_buf();
            tokio::task::spawn_blocking(move || guard::defender_scan(&p)).await.map_err(|e| e.to_string())?
        };
        Ok(Vetting { known, custom, refused, scan, fetched })
    }

    /// What a code's pack holds (before choosing about custom files).
    pub async fn preview(&self, code: &str, cf: Option<Arc<crate::curseforge::Client>>) -> Result<PackPreview, String> {
        let (sess, m) = self.pack_session(code).await?;
        let v = self.vet(&sess, code, &m, cf).await;
        sess.conn.close(0u32.into(), b"done");
        let v = v?;
        Ok(PackPreview { name: m.name.clone(), files: m.files.len(), bytes: m.files.iter().map(|f| f.size).sum(),
            known: v.known, custom: v.custom, refused: v.refused, scan: v.scan })
    }

    /// Install a code's pack as a new profile. Mods come from their sites where possible (checked against the
    /// list), everything else from the sharer, vetted again here (the sharer could change the pack after the
    /// preview). Custom code only with `with_custom`; refused files never. Defender scans the finished profile,
    /// and a threat removes it again.
    pub async fn import(&self, code: &str, with_custom: bool, cf: Option<Arc<crate::curseforge::Client>>,
                        progress: impl Fn(ImportProgress) + Send + Sync + 'static) -> Result<ImportReport, String> {
        let (sess, m) = self.pack_session(code).await?;
        let vet = match self.vet(&sess, code, &m, cf).await {
            Ok(v) => v,
            Err(e) => { sess.conn.close(0u32.into(), b"done"); return Err(e) }
        };
        if let Scan::Threat(t) = &vet.scan {
            sess.conn.close(0u32.into(), b"done");
            return Err(format!("Windows Defender found a threat in this pack's files ({t}). Nothing was installed."));
        }
        let mut left_out: Vec<String> = vet.refused.iter().map(|c| format!("{} (refused: {})", c.path, c.findings.serious.join(", "))).collect();
        let skip: std::collections::HashSet<String> = vet.refused.iter().map(|c| c.path.clone())
            .chain(vet.custom.iter().filter(|_| !with_custom).map(|c| c.path.clone())).collect();
        if !with_custom {
            left_out.extend(vet.custom.iter().map(|c| format!("{} (custom code you chose to leave out)", c.path)));
        }
        let files: Vec<PackFile> = m.files.iter().filter(|f| !skip.contains(&f.path)).cloned().collect();
        let fetched = vet.fetched;
        let left_out = Arc::new(Mutex::new(left_out));
        let (id, game) = packcode::create_instance(&self.instances, &m)?;
        let total = files.len();
        let progress = Arc::new(progress);
        let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let result: Result<(), String> = async {
            // Site downloads, a few at a time; anything that fails comes from the sharer instead.
            let queue = Arc::new(Mutex::new(files.iter().filter(|f| packcode::site_url_ok(&f.url)).cloned().collect::<Vec<_>>()));
            let failed = Arc::new(Mutex::new(Vec::<PackFile>::new()));
            let mut workers = Vec::new();
            for _ in 0..6 {
                let (queue, failed, done, game, progress) = (queue.clone(), failed.clone(), done.clone(), game.clone(), progress.clone());
                workers.push(tokio::task::spawn_blocking(move || loop {
                    let Some(f) = queue.lock().unwrap().pop() else { break };
                    match packcode::from_site(&f).map(|b| packcode::place(&game, &f, &b)) {
                        Some(Ok(())) => {
                            let n = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                            progress(ImportProgress { done: n, total, file: f.path.clone() });
                        }
                        _ => failed.lock().unwrap().push(f),
                    }
                }));
            }
            for w in workers {
                w.await.map_err(|e| e.to_string())?;
            }
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
                    left_out.lock().unwrap().push(format!("{} (refused: {})", f.path, findings.serious.join(", ")));
                    continue;
                }
                packcode::place(&game, &f, &bytes)?;
                let n = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                progress(ImportProgress { done: n, total, file: f.path.clone() });
            }
            for f in rest {
                let bytes = match fetched.get(&f.path) {
                    Some(b) => b.clone(),
                    None => sess.pack_file(code, &f.path).await?,
                };
                packcode::place(&game, &f, &bytes)?;
                let n = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                progress(ImportProgress { done: n, total, file: f.path.clone() });
            }
            Ok(())
        }.await;
        sess.conn.close(0u32.into(), b"done");
        if let Err(e) = result {
            // Don't leave a half-made profile behind.
            let _ = std::fs::remove_dir_all(self.instances.join(&id));
            return Err(e);
        }
        let g = game.clone();
        let scan = tokio::task::spawn_blocking(move || guard::defender_scan(&g)).await.map_err(|e| e.to_string())?;
        if let Scan::Threat(t) = &scan {
            let _ = std::fs::remove_dir_all(self.instances.join(&id));
            return Err(format!("Windows Defender found a threat in the installed pack ({t}). It has been removed."));
        }
        let left_out = left_out.lock().unwrap().clone();
        Ok(ImportReport { id, left_out, scan })
    }

    /// Join a friend's pack from an invite code: creates the profile, downloads the pack, applies it.
    /// Returns the new profile's folder name.
    pub async fn join(&self, code: &str) -> Result<String, String> {
        let inv = share::parse_invite(code)?;
        if let Some(game) = share::find(&self.instances, &inv.pack_id) {
            return Err(format!("You already have this pack ({}).", game.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
        }
        let addr: EndpointAddr = serde_json::from_value(inv.addr.clone()).map_err(|_| "The invite code is damaged.")?;
        let host = addr.id;
        let s = Share { pack_id: inv.pack_id.clone(), name: inv.name.clone(), secret: inv.secret.clone(),
            members: vec![self.me(), Member { id: host.to_string(), name: String::new(), last_seen_ms: now_ms() }], auto_apply: false, created_ms: now_ms() };
        let (sess, _) = self.connect(&s, addr.clone()).await?;
        let files: InstanceReply = serde_json::from_value(sess.call(&Req::Instance).await?).map_err(|e| e.to_string())?;
        sess.conn.close(0u32.into(), b"done");
        let (id, game) = share::create_instance(&self.instances, &s, &files.cfg, &files.pack)?;
        let result = async {
            self.pull(&game, addr).await?;
            share::apply(&game, &[])
        }.await;
        if let Err(e) = result {
            // Don't leave a half-made profile behind.
            let _ = std::fs::remove_dir_all(self.instances.join(&id));
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
}

#[derive(Serialize, Deserialize)]
struct InstanceReply {
    cfg: String,
    pack: String,
}

struct Session {
    conn: Connection,
}

impl Session {
    async fn raw(&self, req: &Req, limit: usize) -> Result<(Resp, Vec<u8>), String> {
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
            if *proof != share::proof(&s.secret, pack, &peer.to_string(), &self.me.to_string()) {
                return (err("wrong pack key"), vec![]);
            }
            let _ = share::merge_members(&game, &[Member { id: peer.to_string(), name: name.clone(), last_seen_ms: now_ms() }]);
            *authed = Some(game);
            let reply = HelloReply { name: self.name.lock().unwrap().clone(), proof: share::proof(&s.secret, pack, &self.me.to_string(), &peer.to_string()) };
            return (ok(serde_json::to_value(reply).unwrap_or_default()), vec![]);
        }
        let Some(game) = authed.as_ref() else { return (err("say hello first"), vec![]) };
        match req {
            Req::Log => {
                let members = share::get(game).map(|s| s.members).unwrap_or_default();
                (ok(serde_json::to_value(LogReply { ids: share::log_ids(game), members }).unwrap_or_default()), vec![])
            }
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
                    Ok((cfg, pack)) => (ok(serde_json::to_value(InstanceReply { cfg, pack }).unwrap_or_default()), vec![]),
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
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            let Ok(bytes) = recv.read_to_end(64 * 1024).await else { break };
            let (head, body, ok) = match serde_json::from_slice::<PackReq>(&bytes) {
                Ok(req) => self.answer(req),
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
        conn.closed().await;
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

        let p = b.preview(&code, None).await.unwrap();
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
        assert!(b.preview(&code, None).await.is_err(), "a withdrawn code serves nothing");
        assert!(a.preview(&code, None).await.unwrap_err().contains("your own code"));
    }
}
