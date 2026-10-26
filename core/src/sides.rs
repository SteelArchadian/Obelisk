//! Which side each mod belongs on (client, server or both), worked out from the jar itself rather
//! than trusting the platforms' tags alone, and the server/client packs built from that.
//!
//! Evidence, strongest first:
//! 1. What the jar declares: fabric.mod.json / quilt.mod.json "environment", Forge's
//!    `clientSideOnly`, `displayTest = "IGNORE_SERVER_VERSION"`, a loader dependency marked
//!    side="CLIENT", or NeoForge `@Mod(dist = Dist.CLIENT)` on every entry class.
//! 2. Well-known client-only mods (by mod id).
//! 3. The platform's tag (Modrinth client_side/server_side, CurseForge Client/Server).
//! 4. Code shape: mixins that only target the client, or nearly every class touching
//!    net/minecraft/client.
//! Then dependencies are settled: anything a server mod requires stays on the server, and a library
//! with no side of its own that only client mods need goes with them.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Both,
    Client,
    Server,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Guess,
    Likely,
    Sure,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModSide {
    pub file_name: String,
    pub name: String,
    pub ids: Vec<String>,
    pub side: Side,
    pub confidence: Confidence,
    /// Plain-language evidence, strongest first.
    pub reasons: Vec<String>,
    /// Display names of server mods that require this one (why it was kept on the server).
    pub needed_by: Vec<String>,
}

/// Everything read out of one jar.
#[derive(Debug, Default, Clone)]
pub struct Facts {
    pub ids: Vec<String>,
    pub name: String,
    pub fabric_env: Option<String>,
    pub forge_client_only: bool,
    pub display_test: Option<String>,
    pub loader_dep_client: bool,
    pub mod_classes: u32,
    pub client_mod_classes: u32,
    pub classes: u32,
    pub client_classes: u32,
    /// Some(true): every mixin config only has client mixins. None: no mixins.
    pub mixins_client_only: Option<bool>,
    /// Required mod ids (loader, Minecraft and Java left out).
    pub deps: Vec<String>,
    /// Mod ids bundled inside (jar-in-jar); they ship with this jar.
    pub bundled: Vec<String>,
}

const PLATFORM_IDS: [&str; 9] = ["minecraft", "neoforge", "forge", "java", "fabricloader", "fabric-loader", "quilt_loader", "quilt_base", "mixinextras"];

/// Mods that only do anything on the client (rendering, sound, HUD, menus, input).
const KNOWN_CLIENT: &[&str] = &[
    "sodium", "embeddium", "rubidium", "iris", "oculus", "sodiumextra", "sodium_extra", "reeses_sodium_options", "sodiumoptionsapi",
    "sodiumdynamiclights", "indium", "entityculling", "entity_model_features", "entity_texture_features", "entity_sound_features",
    "immediatelyfast", "betterf3", "citresewn", "continuity", "lambdynlights", "dynamiclights", "ryoamiclights",
    "notenoughanimations", "skinlayers3d", "waveycapes", "firstperson", "firstpersonmod", "presencefootsteps",
    "sound_physics_remastered", "soundphysics", "ambientsounds", "dynamic_fps", "dynamicfps", "fpsreducer", "betterfpsdist",
    "blur", "controlling", "mousetweaks", "toastcontrol", "chat_heads", "chatheads", "legendarytooltips", "equipmentcompare",
    "iceberg", "prism", "fancymenu", "drippyloadingscreen", "xaerominimap", "xaeroworldmap", "zoomify", "okzoomer", "ok_zoomer",
    "cullleaves", "moreculling", "visuality", "particlerain", "effective", "fallingleaves", "falling_leaves", "reauth", "modmenu",
    "catalogue", "yosbr", "betterthirdperson", "shouldersurfing", "itemborders", "torohealth", "neat", "rrls", "fastquit",
    "betterpingdisplay", "durabilitytooltip", "lootbeams", "particular", "subtle_effects", "chunkanimator", "clienttweaks",
    "euphoria_patcher", "debugify", "screenshot_viewer", "enhanced_boss_bars", "bettermodsbutton", "smoothswapping",
    "auditory", "eatinganimation", "showmeyourskin", "paperdoll", "jeed", "craftpresence", "simplediscordrpc",
    "betteradvancements", "fancy_entity_renderer", "highlight", "make_bubbles_pop", "ears", "dynamiccrosshair",
];

fn read_entry<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Option<Vec<u8>> {
    let mut f = zip.by_name(name).ok()?;
    let mut v = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut v).ok()?;
    Some(v)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// The `dist` list of a class's `@Mod` annotation: None when the class has no `@Mod`, an empty list
/// when `@Mod` has no dist (loads on both sides). Parses just enough of the class file to be exact,
/// since mentioning `Dist.CLIENT` elsewhere in a class says nothing about where the mod loads.
pub fn mod_annotation(b: &[u8]) -> Option<Vec<String>> {
    struct R<'a> {
        b: &'a [u8],
        i: usize,
    }
    impl R<'_> {
        fn u1(&mut self) -> Option<u8> {
            let v = *self.b.get(self.i)?;
            self.i += 1;
            Some(v)
        }
        fn u2(&mut self) -> Option<u16> {
            Some(((self.u1()? as u16) << 8) | self.u1()? as u16)
        }
        fn u4(&mut self) -> Option<u32> {
            Some(((self.u2()? as u32) << 16) | self.u2()? as u32)
        }
        fn skip(&mut self, n: usize) -> Option<()> {
            (self.i + n <= self.b.len()).then(|| self.i += n)
        }
    }
    fn skip_value(r: &mut R, out: &mut Option<&mut Vec<u16>>) -> Option<()> {
        match r.u1()? {
            b'e' => {
                r.skip(2)?;
                let c = r.u2()?;
                if let Some(o) = out {
                    o.push(c);
                }
            }
            b'@' => {
                r.skip(2)?;
                for _ in 0..r.u2()? {
                    r.skip(2)?;
                    skip_value(r, &mut None)?;
                }
            }
            b'[' => {
                for _ in 0..r.u2()? {
                    skip_value(r, out)?;
                }
            }
            _ => r.skip(2)?,
        }
        Some(())
    }

    let mut r = R { b, i: 0 };
    if r.u4()? != 0xCAFE_BABE {
        return None;
    }
    r.skip(4)?;
    let n = r.u2()? as usize;
    let mut utf8: Vec<Option<String>> = vec![None; n];
    let mut k = 1;
    while k < n {
        match r.u1()? {
            1 => {
                let len = r.u2()? as usize;
                let bytes = r.b.get(r.i..r.i + len)?;
                utf8[k] = Some(String::from_utf8_lossy(bytes).into_owned());
                r.skip(len)?;
            }
            3 | 4 => r.skip(4)?,
            5 | 6 => {
                r.skip(8)?;
                k += 1; // longs and doubles take two slots
            }
            7 | 8 | 16 | 19 | 20 => r.skip(2)?,
            9 | 10 | 11 | 12 | 17 | 18 => r.skip(4)?,
            15 => r.skip(3)?,
            _ => return None,
        }
        k += 1;
    }
    let name = |i: u16| utf8.get(i as usize).cloned().flatten().unwrap_or_default();
    r.skip(6)?; // access, this, super
    let ifaces = r.u2()? as usize;
    r.skip(ifaces * 2)?;
    for _ in 0..2 {
        // fields, then methods
        for _ in 0..r.u2()? {
            r.skip(6)?;
            for _ in 0..r.u2()? {
                r.skip(2)?;
                let len = r.u4()? as usize;
                r.skip(len)?;
            }
        }
    }
    for _ in 0..r.u2()? {
        let attr = name(r.u2()?);
        let len = r.u4()? as usize;
        if attr != "RuntimeVisibleAnnotations" {
            r.skip(len)?;
            continue;
        }
        for _ in 0..r.u2()? {
            let ty = name(r.u2()?);
            let is_mod = ty == "Lnet/neoforged/fml/common/Mod;" || ty == "Lnet/minecraftforge/fml/common/Mod;";
            let mut dist: Vec<u16> = Vec::new();
            for _ in 0..r.u2()? {
                let el = name(r.u2()?);
                let mut sink = if is_mod && el == "dist" { Some(&mut dist) } else { None };
                skip_value(&mut r, &mut sink)?;
            }
            if is_mod {
                return Some(dist.into_iter().map(name).collect());
            }
        }
        return None;
    }
    None
}

/// Read everything we can from one mod jar.
pub fn facts(jar: &Path) -> Facts {
    let mut f = Facts::default();
    let Ok(file) = fs::File::open(jar) else { return f };
    let Ok(mut zip) = zip::ZipArchive::new(std::io::BufReader::new(file)) else { return f };
    let text = |zip: &mut zip::ZipArchive<_>, n: &str| read_entry(zip, n).and_then(|b| String::from_utf8(b).ok());
    let mut mixin_configs: Vec<String> = Vec::new();

    // ---- Forge / NeoForge ----
    for toml_name in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
        let Some(doc) = text(&mut zip, toml_name).and_then(|t| t.parse::<toml::Table>().ok()) else { continue };
        if doc.get("clientSideOnly").and_then(|v| v.as_bool()) == Some(true) {
            f.forge_client_only = true;
        }
        if let Some(dt) = doc.get("displayTest").and_then(|v| v.as_str()) {
            f.display_test = Some(dt.to_string());
        }
        for m in doc.get("mods").and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(id) = m.get("modId").and_then(|v| v.as_str()) {
                f.ids.push(id.to_lowercase());
            }
            if f.name.is_empty() {
                f.name = m.get("displayName").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            }
            if let Some(dt) = m.get("displayTest").and_then(|v| v.as_str()) {
                f.display_test = Some(dt.to_string());
            }
        }
        for mx in doc.get("mixins").and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(c) = mx.get("config").and_then(|v| v.as_str()) {
                mixin_configs.push(c.to_string());
            }
        }
        for (_, list) in doc.get("dependencies").and_then(|v| v.as_table()).into_iter().flatten() {
            for d in list.as_array().into_iter().flatten() {
                let id = d.get("modId").and_then(|v| v.as_str()).unwrap_or_default().to_lowercase();
                let side = d.get("side").and_then(|v| v.as_str()).unwrap_or("BOTH").to_uppercase();
                let required = match d.get("type").and_then(|v| v.as_str()) {
                    Some(t) => t.eq_ignore_ascii_case("required"),
                    None => d.get("mandatory").and_then(|v| v.as_bool()).unwrap_or(false),
                };
                if (id == "minecraft" || id == "neoforge" || id == "forge") && side == "CLIENT" {
                    f.loader_dep_client = true;
                }
                if required && side != "CLIENT" && !id.is_empty() && !PLATFORM_IDS.contains(&id.as_str()) {
                    f.deps.push(id);
                }
            }
        }
        break;
    }
    if let Some(manifest) = text(&mut zip, "META-INF/MANIFEST.MF") {
        for line in manifest.lines() {
            if let Some(v) = line.strip_prefix("MixinConfigs:") {
                mixin_configs.extend(v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
            }
        }
    }

    // ---- Fabric / Quilt ----
    if let Some(v) = text(&mut zip, "fabric.mod.json").and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
        if let Some(id) = v["id"].as_str() {
            f.ids.push(id.to_lowercase());
        }
        if f.name.is_empty() {
            f.name = v["name"].as_str().unwrap_or_default().to_string();
        }
        if let Some(env) = v["environment"].as_str() {
            f.fabric_env = Some(env.to_string());
        }
        for (id, _) in v["depends"].as_object().into_iter().flatten() {
            let id = id.to_lowercase();
            if !PLATFORM_IDS.contains(&id.as_str()) {
                f.deps.push(id);
            }
        }
        for m in v["mixins"].as_array().into_iter().flatten() {
            match m {
                serde_json::Value::String(s) => mixin_configs.push(s.clone()),
                serde_json::Value::Object(o) => {
                    if o.get("environment").and_then(|e| e.as_str()) != Some("client") {
                        if let Some(c) = o.get("config").and_then(|c| c.as_str()) {
                            mixin_configs.push(c.to_string());
                        }
                    } else {
                        f.mixins_client_only.get_or_insert(true);
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(v) = text(&mut zip, "quilt.mod.json").and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
        let q = &v["quilt_loader"];
        if let Some(id) = q["id"].as_str() {
            f.ids.push(id.to_lowercase());
        }
        if f.name.is_empty() {
            f.name = q["metadata"]["name"].as_str().unwrap_or_default().to_string();
        }
        match v["minecraft"]["environment"].as_str() {
            Some("client") => f.fabric_env = Some("client".into()),
            Some("dedicated_server") => f.fabric_env = Some("server".into()),
            _ => {}
        }
    }

    // ---- mixin configs: client-only when none of them patch common or server code ----
    for c in &mixin_configs {
        let Some(v) = text(&mut zip, c).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { continue };
        let has = |k: &str| v[k].as_array().is_some_and(|a| !a.is_empty());
        let common = has("mixins") || has("server");
        let client = has("client");
        f.mixins_client_only = Some(match f.mixins_client_only {
            Some(prev) => prev && !common,
            None => !common && client,
        });
    }

    // ---- code: @Mod entry classes and how much of it is client code ----
    let names: Vec<String> = zip.file_names().map(str::to_string).collect();
    for n in &names {
        if (n.starts_with("META-INF/jarjar/") || n.starts_with("META-INF/jars/")) && n.ends_with(".jar") {
            if let Some(bytes) = read_entry(&mut zip, n) {
                f.bundled.extend(crate::configs::ids_in(std::io::Cursor::new(bytes), 1));
            }
            continue;
        }
        if !n.ends_with(".class") || n.starts_with("META-INF/") {
            continue;
        }
        let Some(b) = read_entry(&mut zip, n) else { continue };
        f.classes += 1;
        if contains(&b, b"net/minecraft/client/") {
            f.client_classes += 1;
        }
        if contains(&b, b"Lnet/neoforged/fml/common/Mod;") || contains(&b, b"Lnet/minecraftforge/fml/common/Mod;") {
            if let Some(dist) = mod_annotation(&b) {
                f.mod_classes += 1;
                if !dist.is_empty() && dist.iter().all(|d| d == "CLIENT") {
                    f.client_mod_classes += 1;
                }
            }
        }
    }
    f.ids.sort();
    f.ids.dedup();
    f.deps.sort();
    f.deps.dedup();
    f.deps.retain(|d| !f.ids.contains(d));
    f.bundled.retain(|b| !f.ids.contains(b));
    f
}

/// Decide one mod's side from its own evidence (dependencies are settled later).
fn judge(f: &Facts, platform: &str) -> (Side, Confidence, Vec<String>) {
    let mut why = Vec::new();
    match f.fabric_env.as_deref() {
        Some("client") => return (Side::Client, Confidence::Sure, vec!["The mod says it's client-only (environment: client).".into()]),
        Some("server") => return (Side::Server, Confidence::Sure, vec!["The mod says it's server-only (environment: server).".into()]),
        Some("*") => why.push("The mod says it runs on both sides.".into()),
        _ => {}
    }
    if f.forge_client_only {
        return (Side::Client, Confidence::Sure, vec!["The mod says it's client-only (clientSideOnly).".into()]);
    }
    // displayTest is left out on purpose: mods use IGNORE_SERVER_VERSION for server-side-only
    // features too (world generation, mob behaviour), so it can't tell us the side.
    if f.loader_dep_client {
        return (Side::Client, Confidence::Likely, vec!["The mod only loads on the client (its loader dependency is client-side).".into()]);
    }
    if f.mod_classes > 0 && f.client_mod_classes == f.mod_classes {
        return (Side::Client, Confidence::Sure, vec!["Its entry point only runs on the client (@Mod dist = CLIENT).".into()]);
    }
    if f.ids.iter().any(|id| KNOWN_CLIENT.contains(&id.as_str())) {
        return (Side::Client, Confidence::Likely, vec!["A well-known client-only mod.".into()]);
    }
    if f.mod_classes > f.client_mod_classes && f.client_mod_classes > 0 {
        why.push("Has separate client and common entry points.".into());
    }
    match platform {
        "client" => return (Side::Client, Confidence::Likely, vec!["Its download page lists it as client-only.".into()]),
        "server" => return (Side::Server, Confidence::Likely, vec!["Its download page lists it as server-only.".into()]),
        "both" => why.push("Its download page lists it for client and server.".into()),
        _ => {}
    }
    if f.mixins_client_only == Some(true) && f.classes > 0 && f.client_classes * 10 >= f.classes * 6 {
        return (Side::Client, Confidence::Guess, vec!["All its game patches (mixins) are client-only and most of its code is client code.".into()]);
    }
    if f.classes >= 4 && f.client_classes * 100 >= f.classes * 92 {
        return (Side::Client, Confidence::Guess, vec![format!("{} of its {} classes are client code.", f.client_classes, f.classes)]);
    }
    if why.is_empty() {
        why.push("No sign it's client-only, so it stays on the server to be safe.".into());
    }
    let conf = if f.fabric_env.as_deref() == Some("*") || platform == "both" || why.len() > 1 { Confidence::Likely } else { Confidence::Guess };
    (Side::Both, conf, why)
}

/// Sort every enabled mod in a game folder. `platform` maps file name -> "client" | "server" | "both".
pub fn classify(game: &Path, platform: &HashMap<String, String>) -> Vec<ModSide> {
    let mut jars: Vec<PathBuf> = fs::read_dir(game.join("mods")).into_iter().flatten().flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jar"))
        .collect();
    jars.sort();
    // Reading every class of a few hundred jars is the slow part, so spread it over the cores.
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
    let chunk = jars.len().div_ceil(threads).max(1);
    let all: Vec<Facts> = std::thread::scope(|s| {
        let handles: Vec<_> = jars.chunks(chunk).map(|c| s.spawn(move || c.iter().map(|j| facts(j)).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    settle(&jars, &all, platform)
}

/// True when a jar gave no evidence about its side at all (it is on "both" only by default).
fn no_evidence(f: &Facts, platform: Option<&String>) -> bool {
    f.fabric_env.is_none() && !f.forge_client_only && !f.loader_dep_client && f.client_mod_classes == 0
        && f.mixins_client_only != Some(true) && platform.is_none_or(|p| p.is_empty())
}

/// Judge each jar, then fix dependencies.
pub fn settle(jars: &[PathBuf], all: &[Facts], platform: &HashMap<String, String>) -> Vec<ModSide> {
    let mut out: Vec<ModSide> = jars.iter().zip(all).map(|(j, f)| {
        let file_name = j.file_name().unwrap_or_default().to_string_lossy().to_string();
        let (side, confidence, reasons) = judge(f, platform.get(&file_name).map(String::as_str).unwrap_or(""));
        let name = if f.name.is_empty() || f.name.contains("${") { file_name.trim_end_matches(".jar").to_string() } else { f.name.clone() };
        ModSide { file_name, name, ids: f.ids.clone(), side, confidence, reasons, needed_by: vec![] }
    }).collect();

    // Which jar provides each mod id (bundled ids count for the jar that ships them).
    let mut owner: HashMap<&str, usize> = HashMap::new();
    for (i, f) in all.iter().enumerate() {
        for id in f.ids.iter().chain(&f.bundled) {
            owner.entry(id.as_str()).or_insert(i);
        }
    }

    // A library with no side of its own that only client mods require goes with them.
    for i in 0..out.len() {
        if out[i].side != Side::Both || !no_evidence(&all[i], platform.get(&out[i].file_name)) {
            continue;
        }
        let users: Vec<usize> = (0..all.len()).filter(|&u| u != i && all[u].deps.iter().any(|d| owner.get(d.as_str()) == Some(&i))).collect();
        if !users.is_empty() && users.iter().all(|&u| out[u].side == Side::Client) {
            let names: Vec<String> = users.iter().map(|&u| out[u].name.clone()).collect();
            out[i].side = Side::Client;
            out[i].confidence = Confidence::Likely;
            out[i].reasons = vec![format!("A library only client mods need ({}).", names.join(", "))];
        }
    }

    // Requirements, repeated until nothing changes:
    // - a mod with no side of its own that requires a definitely client-only mod is a client add-on;
    // - otherwise anything a server mod requires stays on the server.
    loop {
        let mut changed = false;
        for i in 0..out.len() {
            if out[i].side == Side::Client {
                continue;
            }
            for d in all[i].deps.clone() {
                let Some(&j) = owner.get(d.as_str()) else { continue };
                if j == i {
                    continue;
                }
                if out[j].side != Side::Client {
                    if out[j].side != Side::Server && !out[j].needed_by.contains(&out[i].name) {
                        let by = out[i].name.clone();
                        out[j].needed_by.push(by);
                    }
                    continue;
                }
                let mostly_client = all[i].classes > 0 && all[i].client_classes * 2 >= all[i].classes;
                if out[j].confidence == Confidence::Sure && mostly_client && no_evidence(&all[i], platform.get(&out[i].file_name)) {
                    out[i].side = Side::Client;
                    out[i].confidence = Confidence::Likely;
                    out[i].reasons = vec![format!("Needs {}, which only runs on the client, and is mostly client code itself.", out[j].name)];
                } else {
                    // Keeping a client mod on the server can crash it, but dropping a mod the server
                    // needs is worse: keep it and say so, so a test start can confirm.
                    let by = out[i].name.clone();
                    let was_sure = out[j].confidence == Confidence::Sure;
                    out[j].side = Side::Both;
                    out[j].confidence = if was_sure { Confidence::Guess } else { Confidence::Likely };
                    out[j].reasons.insert(0, if was_sure {
                        format!("Kept on the server because {by} needs it, though it looks client-only. Check the server starts.")
                    } else {
                        format!("Kept on the server: {by} needs it.")
                    });
                    out[j].needed_by.push(by);
                }
                changed = true;
                break;
            }
        }
        if !changed {
            break;
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

// ---------------------------------------------------------------- building the packs

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PackPlan {
    /// Folder the packs are written into.
    pub out_dir: PathBuf,
    pub name: String,
    pub minecraft: String,
    pub loader: String,
    pub loader_version: String,
    pub max_mem_mb: u32,
    /// Mod file names that go in the server pack.
    pub server_mods: Vec<String>,
    /// Mod file names left out of the client pack (server-only mods).
    pub client_drop: Vec<String>,
    pub make_client: bool,
}

#[derive(Serialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Built {
    pub server_dir: String,
    pub server_zip: String,
    pub client_zip: String,
    pub notes: Vec<String>,
}

/// Folders a server needs from the profile (configs, scripts, data packs). Client-only things
/// (resource packs, shaders, options, saves, logs) are left out.
const SERVER_DIRS: [&str; 8] = ["config", "defaultconfigs", "kubejs", "scripts", "global_packs", "datapacks", "openloader", "paxi"];
/// Never copied into the client pack either: personal or throwaway data.
const CLIENT_SKIP: [&str; 10] = ["saves", "logs", "crash-reports", "screenshots", ".obelisk", "backups", ".cache", "replay_recordings", "local", "debug"];

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<u64> {
    let mut n = 0;
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)?.flatten() {
        let p = e.path();
        let dest = to.join(e.file_name());
        if p.is_dir() {
            n += copy_dir(&p, &dest)?;
        } else {
            fs::copy(&p, &dest)?;
            n += 1;
        }
    }
    Ok(n)
}

fn zip_dir(dir: &Path, zip_path: &Path, prefix: &str, skip: &dyn Fn(&Path) -> bool) -> Result<(), String> {
    let file = fs::File::create(zip_path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).map_err(|e| e.to_string())?.flatten() {
            let p = e.path();
            let rel = p.strip_prefix(dir).unwrap_or(&p);
            if skip(rel) {
                continue;
            }
            let name = format!("{prefix}{}", rel.to_string_lossy().replace('\\', "/"));
            if p.is_dir() {
                z.add_directory(format!("{name}/"), opts).map_err(|e| e.to_string())?;
                stack.push(p);
            } else {
                // Jars are already compressed; storing them is much faster and barely bigger.
                let o = if name.ends_with(".jar") || name.ends_with(".zip") { opts.compression_method(zip::CompressionMethod::Stored) } else { opts };
                z.start_file(name, o).map_err(|e| e.to_string())?;
                let mut src = fs::File::open(&p).map_err(|e| e.to_string())?;
                std::io::copy(&mut src, &mut z).map_err(|e| e.to_string())?;
            }
        }
    }
    z.finish().map_err(|e| e.to_string())?.flush().map_err(|e| e.to_string())
}

/// Java a Minecraft version needs.
pub fn java_for(mc: &str) -> u32 {
    let mut p = mc.split('.').skip(1).map(|s| s.parse::<u32>().unwrap_or(0));
    let (minor, patch) = (p.next().unwrap_or(0), p.next().unwrap_or(0));
    if minor > 20 || (minor == 20 && patch >= 5) { 21 } else if minor >= 18 { 17 } else if minor == 17 { 16 } else { 8 }
}

/// Where the loader's server installer comes from.
pub fn loader_download(loader: &str, mc: &str, version: &str, fabric_installer: &str) -> Option<(String, &'static str)> {
    match loader.to_lowercase().as_str() {
        "neoforge" => Some((format!("https://maven.neoforged.net/releases/net/neoforged/neoforge/{version}/neoforge-{version}-installer.jar"), "installer.jar")),
        "forge" => Some((format!("https://maven.minecraftforge.net/net/minecraftforge/forge/{mc}-{version}/forge-{mc}-{version}-installer.jar"), "installer.jar")),
        "fabric" => Some((format!("https://meta.fabricmc.net/v2/versions/loader/{mc}/{version}/{fabric_installer}/server/jar"), "fabric-server-launch.jar")),
        _ => None,
    }
}

fn scripts(loader: &str, mem: u32) -> Option<(String, String)> {
    let xmx = format!("-Xmx{}M", mem.max(1024));
    match loader.to_lowercase().as_str() {
        "neoforge" | "forge" => Some((
            format!("@echo off\r\ncd /d \"%~dp0\"\r\nif not exist libraries (\r\n  echo Installing the {loader} server. This only happens on the first run...\r\n  java -jar installer.jar --installServer || goto fail\r\n  del installer.jar installer.jar.log 2>nul\r\n  > user_jvm_args.txt echo {xmx}\r\n)\r\ncall run.bat nogui\r\npause\r\nexit /b\r\n:fail\r\necho The installer failed. Is Java installed?\r\npause\r\n"),
            format!("#!/usr/bin/env sh\ncd \"$(dirname \"$0\")\"\nif [ ! -d libraries ]; then\n  echo \"Installing the {loader} server. This only happens on the first run...\"\n  java -jar installer.jar --installServer || exit 1\n  rm -f installer.jar installer.jar.log\n  echo \"{xmx}\" > user_jvm_args.txt\nfi\nexec sh ./run.sh nogui\n"),
        )),
        "fabric" => Some((
            format!("@echo off\r\ncd /d \"%~dp0\"\r\njava {xmx} -jar fabric-server-launch.jar nogui\r\npause\r\n"),
            format!("#!/usr/bin/env sh\ncd \"$(dirname \"$0\")\"\nexec java {xmx} -jar fabric-server-launch.jar nogui\n"),
        )),
        _ => None,
    }
}

/// Build the server folder (and zip), plus the client pack as a Prism import zip.
/// `fetch` downloads a URL (kept outside so the core stays testable offline).
pub fn build(instance_dir: &Path, game: &Path, plan: &PackPlan, fetch: &dyn Fn(&str) -> Result<Vec<u8>, String>) -> Result<Built, String> {
    let mut built = Built::default();
    let safe: String = plan.name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let server = plan.out_dir.join(format!("{safe} Server"));
    if server.exists() {
        return Err(format!("{} already exists. Pick another folder or delete it first.", server.display()));
    }
    fs::create_dir_all(server.join("mods")).map_err(|e| e.to_string())?;

    let wanted: HashSet<&str> = plan.server_mods.iter().map(String::as_str).collect();
    for name in &wanted {
        let src = game.join("mods").join(name);
        if src.is_file() {
            fs::copy(&src, server.join("mods").join(name)).map_err(|e| format!("{name}: {e}"))?;
        }
    }
    for d in SERVER_DIRS {
        let src = game.join(d);
        if src.is_dir() {
            copy_dir(&src, &server.join(d)).map_err(|e| format!("{d}: {e}"))?;
        }
    }

    // Loader and start scripts.
    let fabric_installer = if plan.loader.eq_ignore_ascii_case("fabric") {
        let meta = fetch("https://meta.fabricmc.net/v2/versions/installer")?;
        let v: serde_json::Value = serde_json::from_slice(&meta).map_err(|e| e.to_string())?;
        v.as_array().and_then(|a| a.iter().find(|i| i["stable"].as_bool() == Some(true)).or(a.first()))
            .and_then(|i| i["version"].as_str()).unwrap_or("1.0.1").to_string()
    } else {
        String::new()
    };
    match (loader_download(&plan.loader, &plan.minecraft, &plan.loader_version, &fabric_installer), scripts(&plan.loader, plan.max_mem_mb)) {
        (Some((url, file)), Some((bat, sh))) => {
            let bytes = fetch(&url).map_err(|e| format!("Couldn't download the {} server: {e}", plan.loader))?;
            fs::write(server.join(file), bytes).map_err(|e| e.to_string())?;
            fs::write(server.join("start.bat"), bat).map_err(|e| e.to_string())?;
            fs::write(server.join("start.sh"), sh).map_err(|e| e.to_string())?;
        }
        _ => built.notes.push(format!("{} servers aren't set up automatically yet: install the loader on the server yourself.", plan.loader)),
    }

    let left_out: Vec<String> = fs::read_dir(game.join("mods")).into_iter().flatten().flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".jar") && !wanted.contains(n.as_str()))
        .collect();
    let readme = format!(
        "{name} server\r\n\r\nMinecraft {mc}, {loader} {lv}. Needs Java {java}.\r\n\r\n\
         1. Upload this whole folder to your server (or keep it on this PC).\r\n\
         2. Run start.bat (Windows) or start.sh (Linux/macOS). The first run installs the loader, then stops\r\n   and asks you to accept Minecraft's EULA: open eula.txt and change false to true (https://aka.ms/MinecraftEULA).\r\n\
         3. Run it again. Players join with the client pack.\r\n\r\n\
         Memory: {mem} MB (change it in user_jvm_args.txt).\r\n\r\n\
         Left out because they only run on the client ({n}):\r\n{list}\r\n",
        name = plan.name, mc = plan.minecraft, loader = plan.loader, lv = plan.loader_version, java = java_for(&plan.minecraft),
        mem = plan.max_mem_mb, n = left_out.len(), list = left_out.iter().map(|n| format!("  - {n}")).collect::<Vec<_>>().join("\r\n"),
    );
    fs::write(server.join("README.txt"), readme).map_err(|e| e.to_string())?;

    let server_zip = plan.out_dir.join(format!("{safe} Server.zip"));
    zip_dir(&server, &server_zip, "", &|_| false)?;
    built.server_dir = server.to_string_lossy().to_string();
    built.server_zip = server_zip.to_string_lossy().to_string();

    // Client pack: the Prism instance itself (importable with Prism's Add Instance > Import),
    // minus server-only mods and personal data.
    if plan.make_client {
        let client_zip = plan.out_dir.join(format!("{safe} Client.zip"));
        let game_rel = game.strip_prefix(instance_dir).map(Path::to_path_buf).unwrap_or_default();
        let drop: HashSet<PathBuf> = plan.client_drop.iter().map(|n| game_rel.join("mods").join(n)).collect();
        let skip_dirs: Vec<PathBuf> = CLIENT_SKIP.iter().map(|d| game_rel.join(d)).collect();
        zip_dir(instance_dir, &client_zip, "", &|rel| drop.contains(rel) || skip_dirs.iter().any(|d| rel == d))?;
        built.client_zip = client_zip.to_string_lossy().to_string();
    }
    Ok(built)
}

/// Summary counts, handy for logs and the dialog title.
pub fn counts(mods: &[ModSide]) -> BTreeMap<&'static str, usize> {
    let mut m = BTreeMap::new();
    for s in mods {
        *m.entry(match s.side { Side::Both => "both", Side::Client => "client", Side::Server => "server" }).or_insert(0) += 1;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jar(dir: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let p = dir.join(name);
        let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
        for (n, b) in files {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
        p
    }

    #[test]
    fn reads_the_jars_own_declarations() {
        let d = tempfile::tempdir().unwrap();
        let fab = jar(d.path(), "a.jar", &[("fabric.mod.json", br#"{"id":"zoomy","name":"Zoomy","environment":"client"}"#)]);
        assert_eq!(judge(&facts(&fab), "").0, Side::Client);
        let forge = jar(d.path(), "b.jar", &[("META-INF/neoforge.mods.toml", b"displayTest=\"IGNORE_SERVER_VERSION\"\n[[mods]]\nmodId=\"hud\"\n")]);
        assert_eq!(judge(&facts(&forge), "both").0, Side::Both); // displayTest alone proves nothing
        let client_cls = class_with_mod(Some("CLIENT"));
        let dist = jar(d.path(), "c.jar", &[("META-INF/neoforge.mods.toml", b"[[mods]]\nmodId=\"fx\"\n"), ("fx/Fx.class", &client_cls)]);
        assert_eq!(judge(&facts(&dist), "").0, Side::Client);
        let common_cls = class_with_mod(None);
        let common = jar(d.path(), "d.jar", &[
            ("META-INF/neoforge.mods.toml", b"[[mods]]\nmodId=\"core\"\n"),
            ("core/Core.class", &common_cls),
            ("core/CoreClient.class", &client_cls),
        ]);
        assert_eq!(judge(&facts(&common), "").0, Side::Both);
    }

    /// A minimal real class file: `@Mod("x")`, optionally with `dist = {Dist.<dist>}`.
    fn class_with_mod(dist: Option<&str>) -> Vec<u8> {
        let mut pool: Vec<&str> = vec!["Lnet/neoforged/fml/common/Mod;", "value", "x", "RuntimeVisibleAnnotations", "dist", "Lnet/neoforged/api/distmarker/Dist;"];
        if let Some(d) = dist {
            pool.push(d);
        }
        let mut b: Vec<u8> = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 65];
        b.extend(((pool.len() + 1) as u16).to_be_bytes());
        for s in &pool {
            b.push(1);
            b.extend((s.len() as u16).to_be_bytes());
            b.extend(s.as_bytes());
        }
        b.extend([0, 0x21, 0, 0, 0, 0]); // access, this, super (unused here)
        b.extend([0, 0, 0, 0, 0, 0]); // interfaces, fields, methods
        let mut ann: Vec<u8> = vec![0, 1, 0, 1]; // one annotation, type #1
        ann.extend((if dist.is_some() { 2u16 } else { 1 }).to_be_bytes());
        ann.extend([0, 2, b's', 0, 3]); // value = "x"
        if dist.is_some() {
            ann.extend([0, 5, b'[', 0, 1, b'e', 0, 6, 0, 7]); // dist = { Dist.<7> }
        }
        b.extend([0, 1, 0, 4]); // one attribute: RuntimeVisibleAnnotations
        b.extend((ann.len() as u32).to_be_bytes());
        b.extend(ann);
        b
    }

    #[test]
    fn reads_mod_annotations_exactly() {
        assert_eq!(mod_annotation(&class_with_mod(Some("CLIENT"))), Some(vec!["CLIENT".to_string()]));
        assert_eq!(mod_annotation(&class_with_mod(None)), Some(vec![]));
    }

    #[test]
    fn keeps_what_server_mods_need() {
        let d = tempfile::tempdir().unwrap();
        let lib = jar(d.path(), "lib.jar", &[("META-INF/neoforge.mods.toml", b"[[mods]]\nmodId=\"sharedlib\"\n")]);
        let client = jar(d.path(), "client.jar", &[("fabric.mod.json", br#"{"id":"shiny","environment":"client","depends":{"sharedlib":"*"}}"#)]);
        let server_mod = jar(d.path(), "server.jar", &[("fabric.mod.json", br#"{"id":"machines","depends":{"sharedlib":"*"}}"#)]);
        let jars = vec![lib.clone(), client.clone(), server_mod.clone()];
        let facts_all: Vec<Facts> = jars.iter().map(|j| facts(j)).collect();
        // The platform wrongly tags the library client-only: a server mod needs it, so it stays.
        let platform = HashMap::from([("lib.jar".to_string(), "client".to_string())]);
        let out = settle(&jars, &facts_all, &platform);
        let get = |f: &str| out.iter().find(|m| m.file_name == f).unwrap().clone();
        assert_eq!(get("lib.jar").side, Side::Both);
        assert_eq!(get("lib.jar").needed_by, vec!["server".to_string()]); // named after its jar: it has no display name
        assert_eq!(get("client.jar").side, Side::Client);
    }

    #[test]
    fn library_only_client_mods_need_goes_with_them() {
        let d = tempfile::tempdir().unwrap();
        let lib = jar(d.path(), "lib.jar", &[("META-INF/neoforge.mods.toml", b"[[mods]]\nmodId=\"optionsapi\"\n")]);
        let client = jar(d.path(), "client.jar", &[("fabric.mod.json", br#"{"id":"shiny","environment":"client","depends":{"optionsapi":"*"}}"#)]);
        let jars = vec![lib, client];
        let f: Vec<Facts> = jars.iter().map(|j| facts(j)).collect();
        let out = settle(&jars, &f, &HashMap::new());
        assert!(out.iter().all(|m| m.side == Side::Client));
    }

    #[test]
    fn add_on_of_a_client_mod_is_client() {
        let d = tempfile::tempdir().unwrap();
        let iris = jar(d.path(), "iris.jar", &[("fabric.mod.json", br#"{"id":"iris","environment":"client"}"#)]);
        let toml: &[u8] = b"[[mods]]\nmodId=\"colorwheel\"\n[[dependencies.colorwheel]]\nmodId=\"iris\"\ntype=\"required\"\n";
        let addon = jar(d.path(), "addon.jar", &[("META-INF/neoforge.mods.toml", toml), ("cw/Shader.class", b"net/minecraft/client/renderer")]);
        let jars = vec![iris.clone(), addon];
        let f: Vec<Facts> = jars.iter().map(|j| facts(j)).collect();
        let out = settle(&jars, &f, &HashMap::new());
        assert!(out.iter().all(|m| m.side == Side::Client), "{out:?}");

        // Same dependency from a mod that is mostly game logic: keep both on the server, flagged.
        let logic = jar(d.path(), "logic.jar", &[("META-INF/neoforge.mods.toml", toml), ("cw/Logic.class", b"net/minecraft/world/level")]);
        let jars = vec![iris, logic];
        let f: Vec<Facts> = jars.iter().map(|j| facts(j)).collect();
        let out = settle(&jars, &f, &HashMap::new());
        assert!(out.iter().all(|m| m.side == Side::Both), "{out:?}");
        assert_eq!(out.iter().find(|m| m.file_name == "iris.jar").unwrap().confidence, Confidence::Guess);
    }

    #[test]
    fn java_versions() {
        assert_eq!(java_for("1.21.1"), 21);
        assert_eq!(java_for("1.20.4"), 17);
        assert_eq!(java_for("1.20.6"), 21);
        assert_eq!(java_for("1.16.5"), 8);
    }
}
