//! Will a mod jar actually load in this pack? Reads what the jar itself declares (neoforge.mods.toml, mods.toml,
//! fabric.mod.json) and checks it against the pack's loader, Minecraft version and the mods already there, so an
//! install can pick a version that works and bring what it needs, instead of crashing the game later.

use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct Meta {
    /// Mod ids with their versions (empty when the jar doesn't say).
    pub mods: Vec<(String, String)>,
    pub deps: Vec<Dep>,
    /// "neoforge", "forge" or "fabric": which metadata the jar carries.
    pub kind: String,
    /// A NeoForge/Forge toml without its modLoader line: the loader refuses the jar ("Missing ModLoader").
    pub no_mod_loader: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dep {
    pub id: String,
    pub range: String,
    pub rule: Rule,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rule { Required, Breaks }

/// What the pack offers: its loader, versions and every mod id inside it (with versions).
#[derive(Debug, Clone, Default)]
pub struct Pack {
    pub loader: String,
    pub loader_version: String,
    pub minecraft: String,
    pub mods: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    /// The loader won't load the jar at all (wrong loader, missing modLoader, loader or Minecraft too old/new).
    Unloadable(String),
    /// Needs a mod the pack doesn't have.
    Missing { id: String, range: String },
    /// Needs a newer (or other) version of a mod the pack has.
    Outdated { id: String, have: String, range: String },
    /// Declares itself broken with a mod the pack has.
    Breaks { id: String, have: String, range: String },
}

impl Problem {
    pub fn describe(&self, name: &str) -> String {
        match self {
            Problem::Unloadable(why) => format!("{name} won't load in this pack: {why}"),
            Problem::Missing { id, range } => format!("{name} needs {id}{}", shown(range)),
            Problem::Outdated { id, have, range } => format!("{name} needs {id}{}, but the pack has {have}", shown(range)),
            Problem::Breaks { id, have, range } => format!("{name} says it doesn't work with {id} {have} (it breaks with {range})"),
        }
    }
}

fn shown(range: &str) -> String {
    if range.is_empty() || range == "*" { String::new() } else { format!(" {range}") }
}

pub fn read<R: Read + Seek>(reader: R) -> Option<Meta> {
    let mut zip = zip::ZipArchive::new(reader).ok()?;
    let mut text = |name: &str| -> Option<String> {
        let mut s = String::new();
        zip.by_name(name).ok()?.read_to_string(&mut s).ok()?;
        Some(s)
    };
    let manifest_version = text("META-INF/MANIFEST.MF").and_then(|m| {
        m.lines().find_map(|l| l.strip_prefix("Implementation-Version:").map(|v| v.trim().to_string()))
    });
    for (name, kind) in [("META-INF/neoforge.mods.toml", "neoforge"), ("META-INF/mods.toml", "forge")] {
        let Some(raw) = text(name) else { continue };
        let doc = raw.parse::<toml::Table>().ok()?;
        let mut meta = Meta { kind: kind.into(), no_mod_loader: doc.get("modLoader").and_then(|v| v.as_str()).is_none(), ..Default::default() };
        for m in doc.get("mods").and_then(|v| v.as_array()).into_iter().flatten() {
            let Some(id) = m.get("modId").and_then(|v| v.as_str()) else { continue };
            let mut v = m.get("version").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if v.contains("${") { v = manifest_version.clone().unwrap_or_default() }
            meta.mods.push((id.to_lowercase(), v));
        }
        for list in doc.get("dependencies").and_then(|v| v.as_table()).into_iter().flat_map(|t| t.values()) {
            for d in list.as_array().into_iter().flatten() {
                let Some(id) = d.get("modId").and_then(|v| v.as_str()) else { continue };
                let ty = d.get("type").and_then(|v| v.as_str()).map(str::to_lowercase);
                let mandatory = d.get("mandatory").and_then(|v| v.as_bool());
                let side = d.get("side").and_then(|v| v.as_str()).unwrap_or("BOTH").to_uppercase();
                let rule = match (ty.as_deref(), mandatory) {
                    (Some("required"), _) | (None, Some(true)) => Rule::Required,
                    (Some("incompatible"), _) => Rule::Breaks,
                    _ => continue,
                };
                if side == "SERVER" { continue } // only a dedicated server checks these
                let range = d.get("versionRange").and_then(|v| v.as_str()).unwrap_or("*").to_string();
                meta.deps.push(Dep { id: id.to_lowercase(), range, rule });
            }
        }
        return Some(meta);
    }
    let json: serde_json::Value = serde_json::from_str(&text("fabric.mod.json")?).ok()?;
    let mut meta = Meta { kind: "fabric".into(), ..Default::default() };
    if let Some(id) = json["id"].as_str() {
        meta.mods.push((id.to_lowercase(), json["version"].as_str().unwrap_or("").to_string()));
    }
    for (key, rule) in [("depends", Rule::Required), ("breaks", Rule::Breaks)] {
        for (id, r) in json[key].as_object().into_iter().flatten() {
            let range = match r {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(a) => a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" || "),
                _ => "*".into(),
            };
            meta.deps.push(Dep { id: id.to_lowercase(), range, rule });
        }
    }
    Some(meta)
}

pub fn read_file(jar: &Path) -> Option<Meta> {
    read(std::fs::File::open(jar).ok()?)
}

/// Like `read`, with the mods the jar bundles inside it (META-INF/jarjar) counted as its own.
pub fn read_bundle(bytes: &[u8]) -> Option<Meta> {
    let mut meta = read(std::io::Cursor::new(bytes))?;
    let mut all = HashMap::new();
    collect(std::io::Cursor::new(bytes), &mut all, 0);
    for (id, v) in all {
        if !meta.mods.iter().any(|(i, _)| *i == id) { meta.mods.push((id, v)) }
    }
    Some(meta)
}

/// Every mod id in the pack's enabled jars (bundled ones too), with versions, plus Minecraft and the loader.
pub fn pack(mods_dir: &Path, loader: &str, loader_version: &str, minecraft: &str) -> Pack {
    let mut mods = HashMap::new();
    for e in std::fs::read_dir(mods_dir).into_iter().flatten().flatten() {
        if !e.file_name().to_string_lossy().to_lowercase().ends_with(".jar") { continue }
        let Ok(f) = std::fs::File::open(e.path()) else { continue };
        collect(f, &mut mods, 0);
    }
    Pack { loader: loader.to_lowercase(), loader_version: loader_version.into(), minecraft: minecraft.into(), mods }
}

fn collect<R: Read + Seek>(reader: R, out: &mut HashMap<String, String>, depth: u32) {
    let Ok(mut zip) = zip::ZipArchive::new(reader) else { return };
    let nested: Vec<String> = zip.file_names()
        .filter(|n| (n.starts_with("META-INF/jarjar/") || n.starts_with("META-INF/jars/")) && n.ends_with(".jar"))
        .map(str::to_string).collect();
    let mut bytes_of = |n: &str| -> Option<Vec<u8>> {
        let mut b = Vec::new();
        zip.by_name(n).ok()?.read_to_end(&mut b).ok()?;
        Some(b)
    };
    let inner: Vec<Vec<u8>> = if depth == 0 { nested.iter().filter_map(|n| bytes_of(n)).collect() } else { vec![] };
    drop(bytes_of);
    if let Some(m) = read(&mut zip.into_inner()) {
        for (id, v) in m.mods {
            // A bundled copy may be older than a standalone jar; the newest one is what loads.
            let e = out.entry(id).or_default();
            if e.is_empty() || compare(&v, e) == std::cmp::Ordering::Greater { *e = v }
        }
    }
    for b in inner { collect(std::io::Cursor::new(b), out, depth + 1) }
}

/// Ids the loader itself provides (or Connector stands in for), never a separate download.
fn built_in(id: &str, pack: &Pack) -> Option<String> {
    match id {
        "minecraft" => Some(pack.minecraft.clone()),
        "neoforge" | "forge" | "fml" | "javafml" | "fabricloader" | "fabric-loader" | "quilt_loader" => Some(pack.loader_version.clone()),
        "java" | "connector" | "connectormod" => Some(String::new()),
        // Connector's Forgified Fabric API answers for Fabric API and its modules.
        _ if pack.loader != "fabric" && (id == "fabric" || id == "fabric-api" || id.starts_with("fabric-")) => Some(String::new()),
        _ => None,
    }
}

/// Everything wrong with putting this jar into the pack. `replacing` are mod ids being swapped out by this install.
pub fn check(meta: &Meta, pack: &Pack, replacing: &[String]) -> Vec<Problem> {
    let mut out = Vec::new();
    let loader = pack.loader.as_str();
    match (meta.kind.as_str(), loader) {
        ("neoforge", "neoforge") | ("forge", "forge") => {
            if meta.no_mod_loader {
                out.push(Problem::Unloadable("its mods.toml has no modLoader line, so the loader rejects it".into()));
            }
        }
        ("forge", "neoforge") => out.push(Problem::Unloadable("it's a Forge build, not a NeoForge one".into())),
        ("neoforge", "forge") => out.push(Problem::Unloadable("it's a NeoForge build, not a Forge one".into())),
        ("fabric", "fabric" | "quilt" | "neoforge") => {} // NeoForge loads Fabric jars through Connector
        ("neoforge" | "forge", "fabric" | "quilt") => out.push(Problem::Unloadable(format!("it's a {} build", meta.kind))),
        _ => {}
    }
    let mine: Vec<&str> = meta.mods.iter().map(|(id, _)| id.as_str()).collect();
    for d in &meta.deps {
        if mine.contains(&d.id.as_str()) { continue }
        let builtin = built_in(&d.id, pack);
        let have = builtin.clone().or_else(|| if replacing.contains(&d.id) { None } else { pack.mods.get(&d.id).cloned() });
        match (d.rule, have) {
            (Rule::Required, None) => out.push(Problem::Missing { id: d.id.clone(), range: d.range.clone() }),
            (Rule::Required, Some(v)) => {
                if !v.is_empty() && !satisfies(&v, &d.range, &meta.kind) {
                    if builtin.is_some() && matches!(d.id.as_str(), "minecraft" | "neoforge" | "forge" | "fabricloader") {
                        let what = if d.id == "minecraft" { "Minecraft" } else { "the loader" };
                        out.push(Problem::Unloadable(format!("it needs {what} {}, and the pack has {v}", d.range)));
                    } else if builtin.is_none() {
                        out.push(Problem::Outdated { id: d.id.clone(), have: v, range: d.range.clone() });
                    }
                }
            }
            (Rule::Breaks, Some(v)) if builtin.is_none() => {
                if satisfies(&v, &d.range, &meta.kind) {
                    out.push(Problem::Breaks { id: d.id.clone(), have: v, range: d.range.clone() });
                }
            }
            _ => {}
        }
    }
    out
}

// ---- Versions and ranges ----

/// Compare two version strings: numbers numerically, a pre-release ("-beta.2") before its release,
/// build metadata ("+mc1.21.1") ignored.
pub fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    let split = |s: &str| -> (Vec<u64>, String) {
        let s = s.split('+').next().unwrap_or("").trim();
        let (core, pre) = match s.find('-') { Some(i) => (&s[..i], s[i + 1..].to_string()), None => (s, String::new()) };
        let nums = core.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(|p| p.parse().unwrap_or(0)).collect();
        (nums, pre)
    };
    let ((na, pa), (nb, pb)) = (split(a), split(b));
    for i in 0..na.len().max(nb.len()) {
        match na.get(i).unwrap_or(&0).cmp(nb.get(i).unwrap_or(&0)) {
            Equal => continue,
            o => return o,
        }
    }
    match (pa.is_empty(), pb.is_empty()) {
        (true, true) => Equal,
        (true, false) => Greater,
        (false, true) => Less,
        (false, false) => pa.cmp(&pb),
    }
}

/// Like `compare`, but with the qualifier order NeoForge and Forge use (Maven's): alpha, beta, milestone, rc and
/// snapshot come before the release, and any other word after it ("1.2.0-neoforge" is newer than "1.2.0").
fn compare_maven(a: &str, b: &str) -> std::cmp::Ordering {
    let qualifier = |s: &str| -> String {
        let s = s.split('+').next().unwrap_or("").trim();
        s.find('-').map(|i| s[i + 1..].to_lowercase()).unwrap_or_default()
    };
    let rank = |q: &str| -> u8 {
        let word: String = q.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        match word.as_str() {
            "alpha" | "a" => 0,
            "beta" | "b" => 1,
            "milestone" | "m" => 2,
            "rc" | "cr" => 3,
            "snapshot" => 4,
            "" if q.is_empty() => 5,
            "ga" | "final" | "release" => 5,
            "sp" => 6,
            _ => 7,
        }
    };
    let core = |s: &str| s.split('+').next().unwrap_or("").split('-').next().unwrap_or("").to_string();
    let (qa, qb) = (qualifier(a), qualifier(b));
    compare(&core(a), &core(b)).then_with(|| rank(&qa).cmp(&rank(&qb))).then_with(|| qa.cmp(&qb))
}

/// Does `v` satisfy a range: Maven style ("[1.8.0,)", "[1.21,1.21.2)") for NeoForge/Forge,
/// semver style (">=0.6 <0.8", "~1.2", "^1.2", "1.21.x", alternatives joined by "||") for Fabric.
pub fn satisfies(v: &str, range: &str, kind: &str) -> bool {
    let range = range.trim();
    if range.is_empty() || range == "*" { return true }
    if kind != "fabric" && (range.starts_with('[') || range.starts_with('(')) {
        return maven(v, range);
    }
    if kind != "fabric" {
        return true; // a bare Maven version is only a recommendation
    }
    range.split("||").any(|alt| alt.split_whitespace().all(|c| semver_part(v, c)))
}

fn maven(v: &str, range: &str) -> bool {
    // "[1,2),[3,4)": any of the comma-joined groups.
    let mut groups = Vec::new();
    let mut cur = String::new();
    for ch in range.chars() {
        cur.push(ch);
        if ch == ']' || ch == ')' { groups.push(std::mem::take(&mut cur)) }
    }
    groups.iter().map(|g| g.trim_start_matches(',').trim()).filter(|g| !g.is_empty()).any(|g| {
        let (lo_inc, hi_inc) = (g.starts_with('['), g.ends_with(']'));
        let inner = &g[1..g.len() - 1];
        let (lo, hi) = match inner.split_once(',') { Some((l, h)) => (l.trim(), h.trim()), None => (inner.trim(), inner.trim()) };
        use std::cmp::Ordering::*;
        let lo_ok = lo.is_empty() || matches!((compare_maven(v, lo), lo_inc), (Greater, _) | (Equal, true));
        let hi_ok = hi.is_empty() || matches!((compare_maven(v, hi), hi_inc), (Less, _) | (Equal, true));
        lo_ok && hi_ok
    })
}

fn semver_part(v: &str, c: &str) -> bool {
    use std::cmp::Ordering::*;
    let c = c.trim();
    if c.is_empty() || c == "*" { return true }
    for (op, test) in [(">=", &[Greater, Equal][..]), ("<=", &[Less, Equal]), (">", &[Greater]), ("<", &[Less]), ("=", &[Equal])] {
        if let Some(rest) = c.strip_prefix(op) { return test.contains(&compare(v, rest.trim())) }
    }
    let nums = |s: &str| s.split(['.', '-', '+']).take_while(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty()).map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    if let Some(rest) = c.strip_prefix('~') {
        // ~1.2.3 means >=1.2.3 and <1.3
        let n = nums(rest);
        let upper = if n.len() >= 2 { format!("{}.{}", n[0], n[1] + 1) } else { format!("{}", n.first().unwrap_or(&0) + 1) };
        return compare(v, rest) != Less && compare(v, &upper) == Less;
    }
    if let Some(rest) = c.strip_prefix('^') {
        let n = nums(rest);
        let upper = format!("{}", n.first().unwrap_or(&0) + 1);
        return compare(v, rest) != Less && compare(v, &upper) == Less;
    }
    if c.contains(".x") || c.contains(".X") || c.contains(".*") {
        let prefix: Vec<&str> = c.split('.').take_while(|p| !matches!(*p, "x" | "X" | "*")).collect();
        let parts: Vec<&str> = v.split(['.', '-', '+']).collect();
        return prefix.iter().enumerate().all(|(i, p)| parts.get(i) == Some(p));
    }
    compare(v, c) == Equal
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_loader_tag_after_the_version_is_newer_not_a_pre_release() {
        assert!(super::satisfies("1.2.0-neoforge+mc1.21.1", "[1.2.0,)", "neoforge"));
        assert!(!super::satisfies("1.1.2-neoforge+mc1.21.1", "[1.2.0,)", "neoforge"));
        assert!(!super::satisfies("1.2.0-beta.3", "[1.2.0,)", "neoforge"));
        assert!(super::satisfies("1.2.0-rc1", "[1.1,1.2.0)", "neoforge"));
    }

    use super::*;
    use std::io::Write;

    fn jar(files: &[(&str, &str)]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (n, t) in files {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(t.as_bytes()).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    fn neo_pack() -> Pack {
        Pack { loader: "neoforge".into(), loader_version: "21.1.252".into(), minecraft: "1.21.1".into(),
            mods: [("lithostitched", "1.8.0"), ("sodium", "0.8.0"), ("iris", "1.8.14")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect() }
    }

    #[test]
    fn ranges() {
        assert!(satisfies("21.1.264", "[21.1.264,)", "neoforge"));
        assert!(!satisfies("21.1.252", "[21.1.264,)", "neoforge"));
        assert!(satisfies("1.21.1", "[1.21,1.21.2)", "neoforge"));
        assert!(!satisfies("1.21.2", "[1.21,1.21.2)", "neoforge"));
        assert!(satisfies("0.7.3", ">=0.6 <0.8", "fabric"));
        assert!(!satisfies("0.8.0", ">=0.6 <0.8", "fabric"));
        assert!(satisfies("1.21.1", "1.21.x", "fabric"));
        assert!(satisfies("1.2.9", "~1.2.3", "fabric") && !satisfies("1.3.0", "~1.2.3", "fabric"));
        assert!(satisfies("0.8.0", "<0.7 || >=0.8", "fabric"));
        assert!(compare("1.0.0-beta.1", "1.0.0") == std::cmp::Ordering::Less);
        assert!(compare("1.8.1+mc1.21.1", "1.8.0") == std::cmp::Ordering::Greater);
    }

    #[test]
    fn the_five_odyssey_crashes_are_caught() {
        let pack = neo_pack();
        // 1. Needs a newer NeoForge than the pack has.
        let eco = read(std::io::Cursor::new(jar(&[("META-INF/neoforge.mods.toml",
            "modLoader=\"javafml\"\nloaderVersion=\"[4,)\"\n[[mods]]\nmodId=\"ecologics\"\nversion=\"2.3.8\"\n[[dependencies.ecologics]]\nmodId=\"neoforge\"\ntype=\"required\"\nversionRange=\"[21.1.264,)\"\n")]))).unwrap();
        assert!(matches!(check(&eco, &pack, &[]).as_slice(), [Problem::Unloadable(_)]));
        // 2. A required library that isn't in the pack.
        let yung = read(std::io::Cursor::new(jar(&[("META-INF/neoforge.mods.toml",
            "modLoader=\"javafml\"\n[[mods]]\nmodId=\"yungscavebiomes\"\nversion=\"3.1.1\"\n[[dependencies.yungscavebiomes]]\nmodId=\"yungsapi\"\ntype=\"required\"\nversionRange=\"[5.1,)\"\n")]))).unwrap();
        assert_eq!(check(&yung, &pack, &[]), vec![Problem::Missing { id: "yungsapi".into(), range: "[5.1,)".into() }]);
        // 3. No modLoader line.
        let towers = read(std::io::Cursor::new(jar(&[("META-INF/neoforge.mods.toml", "[[mods]]\nmodId=\"structory_towers\"\nversion=\"1.0.17\"\n")]))).unwrap();
        assert!(matches!(check(&towers, &pack, &[]).as_slice(), [Problem::Unloadable(_)]));
        // 4. A library the pack has, but too old for it.
        let ru = read(std::io::Cursor::new(jar(&[("META-INF/neoforge.mods.toml",
            "modLoader=\"javafml\"\n[[mods]]\nmodId=\"regions_unexplored\"\nversion=\"0.6.3\"\n[[dependencies.regions_unexplored]]\nmodId=\"lithostitched\"\ntype=\"required\"\nversionRange=\"[1.8.1,)\"\n")]))).unwrap();
        assert!(matches!(check(&ru, &pack, &[]).as_slice(), [Problem::Outdated { .. }]));
        // 5. A Fabric mod (through Connector) that says it breaks with the pack's Sodium.
        let ip = read(std::io::Cursor::new(jar(&[("fabric.mod.json",
            r#"{"id":"imm_ptl_core","version":"6.0.6","depends":{"fabricloader":">=0.15","minecraft":"1.21.1","fabric-api":"*"},"breaks":{"sodium":">=0.6"}}"#)]))).unwrap();
        assert!(matches!(check(&ip, &pack, &[]).as_slice(), [Problem::Breaks { .. }]));
        // And a jar that fits raises nothing.
        let ok = read(std::io::Cursor::new(jar(&[("META-INF/neoforge.mods.toml",
            "modLoader=\"javafml\"\n[[mods]]\nmodId=\"fine\"\nversion=\"${file.jarVersion}\"\n[[dependencies.fine]]\nmodId=\"neoforge\"\ntype=\"required\"\nversionRange=\"[21.1,)\"\n[[dependencies.fine]]\nmodId=\"minecraft\"\ntype=\"required\"\nversionRange=\"[1.21.1,1.22)\"\n"),
            ("META-INF/MANIFEST.MF", "Implementation-Version: 2.0\n")]))).unwrap();
        assert_eq!(ok.mods, vec![("fine".to_string(), "2.0".to_string())]);
        assert!(check(&ok, &pack, &[]).is_empty());
    }
}
