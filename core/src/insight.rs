//! The second half of diagnosis: who's who in a log (logger names and code packages → installed
//! mods), Obelisk's knowledge base of known problems and harmless noise (knowledge.json), where the
//! time went during start-up and world loading, and errors grouped by the mod they belong to.

use crate::crash::{Diagnosis, Finding, Suspect};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;

// ---- who's who

/// Installed mods by id, display name and code package.
#[derive(Default, Debug)]
pub struct Index {
    /// lowercase id or display name → (display name, file)
    names: HashMap<String, (String, String)>,
    /// "dev.chill.shaderengine" → (display name, file); prefixes shared by two jars are dropped.
    packages: HashMap<String, (String, String)>,
    pub ids: Vec<String>,
}

impl Index {
    pub fn build(game: &Path) -> Index {
        let mut ix = Index::default();
        let mut pkg_owner: HashMap<String, Option<(String, String)>> = HashMap::new();
        let Ok(entries) = fs::read_dir(game.join("mods")) else { return ix };
        for e in entries.flatten() {
            let file = e.file_name().to_string_lossy().into_owned();
            if !file.ends_with(".jar") {
                continue;
            }
            let Ok(f) = fs::File::open(e.path()) else { continue };
            let Ok(mut zip) = zip::ZipArchive::new(std::io::BufReader::new(f)) else { continue };
            let mut mods: Vec<(String, String)> = Vec::new();
            for meta in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
                if let Some(s) = read(&mut zip, meta) {
                    if let Ok(v) = s.parse::<toml::Table>() {
                        for m in v.get("mods").and_then(|m| m.as_array()).into_iter().flatten() {
                            let id = m.get("modId").and_then(|x| x.as_str()).unwrap_or("").to_string();
                            let name = m.get("displayName").and_then(|x| x.as_str()).unwrap_or(&id).to_string();
                            if !id.is_empty() {
                                mods.push((id, name));
                            }
                        }
                    }
                    break;
                }
            }
            if mods.is_empty() {
                for meta in ["fabric.mod.json", "quilt.mod.json"] {
                    if let Some(v) = read(&mut zip, meta).and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()) {
                        let q = &v["quilt_loader"];
                        let id = v["id"].as_str().or(q["id"].as_str()).unwrap_or("").to_string();
                        let name = v["name"].as_str().or(q["metadata"]["name"].as_str()).unwrap_or(&id).to_string();
                        if !id.is_empty() {
                            mods.push((id, name));
                        }
                    }
                }
            }
            let Some((_, main_name)) = mods.first().cloned() else { continue };
            for (id, name) in &mods {
                ix.ids.push(id.to_lowercase());
                ix.names.entry(id.to_lowercase()).or_insert((name.clone(), file.clone()));
                ix.names.entry(name.to_lowercase()).or_insert((name.clone(), file.clone()));
            }
            let mut seen = std::collections::HashSet::new();
            for i in 0..zip.len() {
                let Some(n) = zip.name_for_index(i) else { continue };
                if !n.ends_with(".class") || n.starts_with("META-INF") {
                    continue;
                }
                let parts: Vec<&str> = n.split('/').collect();
                if parts.len() < 4 {
                    continue;
                }
                let p = parts[..3].join(".");
                if seen.insert(p.clone()) {
                    let owner = (main_name.clone(), file.clone());
                    pkg_owner.entry(p).and_modify(|o| if o.as_ref() != Some(&owner) { *o = None }).or_insert(Some(owner));
                }
            }
        }
        ix.packages = pkg_owner.into_iter().filter_map(|(k, v)| v.map(|v| (k, v))).collect();
        ix
    }

    pub fn has(&self, id: &str) -> bool {
        self.ids.iter().any(|i| i == id)
    }

    /// The mod behind a logger name, class name or resource namespace.
    pub fn who(&self, name: &str) -> Option<(String, String)> {
        let n = name.trim();
        if let Some(v) = self.names.get(&n.to_lowercase()) {
            return Some(v.clone());
        }
        let parts: Vec<&str> = n.split('.').collect();
        if parts.len() >= 3 {
            if let Some(v) = self.packages.get(&parts[..3].join(".")) {
                return Some(v.clone());
            }
        }
        None
    }
}

fn read<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    let mut f = zip.by_name(name).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    Some(s)
}

fn is_vanilla(logger: &str) -> bool {
    let l = logger.to_lowercase();
    l.starts_with("net.minecraft") || l.starts_with("com.mojang") || l == "minecraft" || l.starts_with("net.neoforged") || l == "neoforge"
        || l.starts_with("net.fabricmc") || l == "fml" || l == "mixin"
}

/// A friendly name for a log line's source.
fn source(ix: &Index, logger: &str) -> (String, String) {
    if logger.is_empty() || is_vanilla(logger) {
        return ("Minecraft".into(), String::new());
    }
    ix.who(logger).unwrap_or_else(|| (logger.rsplit('.').next().unwrap_or(logger).to_string(), String::new()))
}

// ---- knowledge base

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Rule {
    id: String,
    kinds: Vec<String>,
    all: Vec<String>,
    any: Vec<String>,
    not: Vec<String>,
    mods: Vec<String>,
    any_mods: Vec<String>,
    not_mods: Vec<String>,
    severity: String,
    title: String,
    detail: String,
    fix: String,
    action: String,
    blame: String,
    blame_from: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Slow {
    who: String,
    any: Vec<String>,
    note: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Noise {
    any: Vec<String>,
    all: Vec<String>,
    label: String,
    blame_from: Vec<String>,
}

#[derive(Deserialize)]
struct Knowledge {
    problems: Vec<Rule>,
    slow: Vec<Slow>,
    noise: Vec<Noise>,
}

fn knowledge() -> &'static Knowledge {
    static K: std::sync::OnceLock<Knowledge> = std::sync::OnceLock::new();
    K.get_or_init(|| serde_json::from_str(include_str!("knowledge.json")).expect("knowledge.json"))
}

/// "nomadictents" from "…advancement nomadictents:recipes/x…" after a marker.
fn namespace_after(text: &str, lower: &str, markers: &[String]) -> Option<String> {
    for m in markers {
        if let Some(i) = lower.find(m.as_str()) {
            let rest = &text[i + m.len()..];
            let word: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')).collect();
            let ns = word.split(':').next().unwrap_or("").trim_matches('.').to_lowercase();
            if !ns.is_empty() && ns != "minecraft" {
                return Some(ns);
            }
        }
    }
    None
}

fn add_suspect(d: &mut Diagnosis, ix: &Index, id: &str, reason: &str, score: u32) {
    let file = ix.who(id).map(|w| w.1).unwrap_or_default();
    if let Some(s) = d.suspects.iter_mut().find(|s| s.mod_id == id) {
        s.score += score;
        s.reason = reason.into();
    } else {
        d.suspects.push(Suspect { mod_id: id.into(), file_name: file, reason: reason.into(), score });
    }
    d.suspects.sort_by(|a, b| b.score.cmp(&a.score));
}

/// Known problems from the knowledge base, added to the findings (and suspects).
pub fn apply_knowledge(d: &mut Diagnosis, kind: &str, text: &str, ix: &Index) {
    let lower = text.to_lowercase();
    for r in &knowledge().problems {
        if !r.kinds.is_empty() && !r.kinds.iter().any(|k| k == kind) {
            continue;
        }
        let ok = r.all.iter().all(|s| lower.contains(s.as_str()))
            && (r.any.is_empty() || r.any.iter().any(|s| lower.contains(s.as_str())))
            && !r.not.iter().any(|s| lower.contains(s.as_str()))
            && r.mods.iter().all(|m| ix.has(m))
            && (r.any_mods.is_empty() || r.any_mods.iter().any(|m| ix.has(m)))
            && !r.not_mods.iter().any(|m| ix.has(m));
        if !ok || d.findings.iter().any(|f| f.title == r.title) {
            continue;
        }
        let blame = if !r.blame.is_empty() { Some(r.blame.clone()) } else { namespace_after(text, &lower, &r.blame_from) };
        let mut detail = r.detail.clone();
        if let Some(b) = &blame {
            let name = ix.who(b).map(|w| w.0).unwrap_or_else(|| b.clone());
            if r.blame.is_empty() {
                detail = format!("{detail} It points at {name}.");
            }
            if r.severity != "info" {
                add_suspect(d, ix, b, &format!("Known problem: {}", r.title.to_lowercase()), 50);
            }
        }
        let target = blame.as_ref().and_then(|b| ix.who(b)).map(|w| w.1).unwrap_or_default();
        d.findings.push(Finding {
            severity: r.severity.clone(),
            title: r.title.clone(),
            detail,
            fix: r.fix.clone(),
            action: if !r.action.is_empty() { r.action.clone() } else if !target.is_empty() && r.severity == "critical" { "disable".into() } else { String::new() },
            target: if r.action.is_empty() { target } else { String::new() },
            lines: matching_lines(text, r.any.iter().chain(&r.all)),
        });
        let _ = &r.id;
    }
}

/// Up to 12 lines of the file containing any of the patterns.
fn matching_lines<'a>(text: &str, pats: impl Iterator<Item = &'a String>) -> Vec<String> {
    let pats: Vec<&String> = pats.filter(|p| p.len() >= 4).collect();
    if pats.is_empty() {
        return Vec::new();
    }
    text.lines().filter(|l| { let l = l.to_lowercase(); pats.iter().any(|p| l.contains(p.as_str())) })
        .take(12).map(|l| l.chars().take(400).collect()).collect()
}

/// After the generic findings are made, attach the lines behind the ones that didn't get any.
pub fn attach_lines(d: &mut Diagnosis, text: &str) {
    let lag: Vec<String> = text.lines().filter(|l| l.contains("Can't keep up!")).take(12).map(String::from).collect();
    let fallback: Vec<String> = d.excerpt.iter().take(12).cloned().collect();
    for f in d.findings.iter_mut().filter(|f| f.lines.is_empty()) {
        f.lines = if f.title.starts_with("The world lagged") { lag.clone() }
            else if f.title.contains("error line") { d.excerpt.iter().take(30).cloned().collect() }
            else { fallback.clone() };
    }
}

// ---- log lines

struct Line<'a> {
    t: Option<f64>,
    level: &'a str,
    logger: &'a str,
    msg: &'a str,
    raw: &'a str,
}

/// "[20Oct2026 21:21:02.297] [Thread/LEVEL] [logger/marker]: msg" or "[21:21:02] [Thread/LEVEL]: msg"
fn parse(l: &str) -> Line<'_> {
    let mut out = Line { t: None, level: "", logger: "", msg: l, raw: l };
    let Some(rest) = l.strip_prefix('[') else { return out };
    let Some(end) = rest.find(']') else { return out };
    let stamp = &rest[..end];
    let time = stamp.rsplit(' ').next().unwrap_or("");
    let p: Vec<&str> = time.split([':', '.']).collect();
    if p.len() >= 3 {
        if let (Ok(h), Ok(m), Ok(s)) = (p[0].parse::<f64>(), p[1].parse::<f64>(), p[2].parse::<f64>()) {
            let ms = p.get(3).and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0) / 1000.0;
            out.t = Some(h * 3600.0 + m * 60.0 + s + ms);
        }
    }
    let rest = &rest[end + 1..];
    if let Some(a) = rest.find(" [") {
        let r2 = &rest[a + 2..];
        if let Some(b) = r2.find(']') {
            out.level = r2[..b].rsplit('/').next().unwrap_or("");
            let r3 = &r2[b + 1..];
            if let Some(c) = r3.strip_prefix(" [") {
                if let Some(e) = c.find("]: ") {
                    out.logger = c[..e].split('/').next().unwrap_or("");
                    out.msg = &c[e + 3..];
                    return out;
                }
            }
            out.msg = r3.trim_start_matches(':').trim();
        }
    }
    out
}

#[derive(Serialize, Debug, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SlowStep {
    pub seconds: f32,
    /// "startup", "world" or "shutdown"
    pub phase: String,
    pub who: String,
    pub file: String,
    /// What the knowledge base says about it, if anything.
    pub note: String,
    pub before: String,
    pub after: String,
}

#[derive(Serialize, Debug, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModTime {
    pub who: String,
    pub file: String,
    pub seconds: f32,
}

#[derive(Serialize, Debug, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    /// Seconds from launch to the title screen.
    pub startup_s: f32,
    /// Seconds from choosing a world to being in it.
    pub world_s: f32,
    pub steps: Vec<SlowStep>,
    pub by_mod: Vec<ModTime>,
}

#[derive(Serialize, Debug, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ErrorGroup {
    pub who: String,
    pub file: String,
    pub count: u32,
    pub sample: String,
}

#[derive(Serialize, Debug, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NoiseGroup {
    pub label: String,
    pub count: u32,
    /// Mods the noise comes from, most first.
    pub from: Vec<String>,
}

fn slow_note(who: &str, before: &str, after: &str) -> String {
    let both = format!("{} {}", before.to_lowercase(), after.to_lowercase());
    knowledge().slow.iter()
        .find(|s| s.who.eq_ignore_ascii_case(who) && (s.any.is_empty() || s.any.iter().any(|a| both.contains(a.as_str()))))
        .map(|s| s.note.clone()).unwrap_or_default()
}

/// Where the time went: silent stretches of the log during start-up, world loading and shutdown,
/// put down to the mod that was working (the one that logged right after the silence, or both
/// sides when it's the same mod). Quiet time in menus or in game is not counted.
pub fn timeline(text: &str, ix: &Index) -> Timeline {
    let lines: Vec<Line> = text.lines().map(parse).filter(|l| l.t.is_some()).collect();
    let mut tl = Timeline::default();
    let Some(first) = lines.first().and_then(|l| l.t) else { return tl };
    let mut phase = "startup";
    let mut world_start: Option<f64> = None;
    let mut prev: Option<&Line> = None;
    let mut day = 0.0;
    let mut last_t = first;
    let mut by: HashMap<(String, String), f32> = HashMap::new();
    for l in &lines {
        let mut t = l.t.unwrap() + day;
        if t + 43_200.0 < last_t {
            day += 86_400.0;
            t += 86_400.0;
        }
        let m = l.msg.to_lowercase();
        if let Some(p) = prev {
            let gap = (t - last_t) as f32;
            if gap >= 1.0 && phase != "menu" && phase != "game" {
                let (a, af) = source(ix, p.logger);
                let (b, bf) = source(ix, l.logger);
                let (who, file) = if a == b || b == "Minecraft" && a != "Minecraft" { (a, af) } else { (b, bf) };
                *by.entry((who.clone(), file.clone())).or_default() += gap;
                if gap >= 2.0 {
                    let note = slow_note(&who, p.msg, l.msg);
                    tl.steps.push(SlowStep { seconds: gap, phase: phase.into(), who, file, note, before: short(p.raw), after: short(l.raw) });
                }
            }
        }
        // Phase changes, after counting the gap that led up to this line.
        // Start-up ends at the title screen. ModernFix's "Game took N seconds" counts from the real
        // launch (before the log starts), so it wins over our own clock when it's there.
        if m.starts_with("game took") && world_start.is_none() {
            if let Some(n) = m.split_whitespace().nth(2).and_then(|n| n.parse().ok()) {
                tl.startup_s = n;
            }
            if phase == "startup" {
                phase = "menu";
            }
        }
        if phase == "startup" && (m.contains("[timeline] titlescreen") || m.contains("[timeline] titleframe")) {
            tl.startup_s = m.split(" at ").nth(1).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse::<f32>().ok()).map(|ms| ms / 1000.0).unwrap_or((t - first) as f32);
            phase = "menu";
        }
        // Opening a world: singleplayer starts its server; multiplayer logs "Connecting to host, port".
        let opening = m.contains("starting integrated minecraft server") || m.contains("[timeline] stemstart")
            || phase == "menu" && (m.contains("preparing level") || m.starts_with("connecting to ") && m.contains(", "));
        if (phase == "menu" || phase == "startup") && opening {
            if phase == "startup" && tl.startup_s == 0.0 {
                tl.startup_s = (t - first) as f32;
            }
            phase = "world";
            world_start.get_or_insert(t);
        }
        if phase == "world" && (m.contains("logged in with entity id") || m.contains("joined the game")) {
            tl.world_s = world_start.map(|w| (t - w) as f32).unwrap_or(0.0);
            phase = "game";
        }
        if m.starts_with("stopping!") || m.contains("stopping server") {
            phase = "shutdown";
        }
        last_t = t;
        prev = Some(l);
    }
    tl.steps.sort_by(|a, b| b.seconds.total_cmp(&a.seconds));
    tl.steps.truncate(12);
    let mut v: Vec<ModTime> = by.into_iter().filter(|(_, s)| *s >= 2.0).map(|((who, file), seconds)| ModTime { who, file, seconds }).collect();
    v.sort_by(|a, b| b.seconds.total_cmp(&a.seconds));
    v.truncate(10);
    tl.by_mod = v;
    tl
}

fn short(l: &str) -> String {
    let s: String = l.chars().take(220).collect();
    s
}

/// The message with numbers and ids blurred, so repeats of one problem group together.
fn template(msg: &str) -> String {
    let mut out = String::new();
    let mut digit = false;
    for c in msg.chars().take(140) {
        if c.is_ascii_digit() {
            if !digit {
                out.push('#');
            }
            digit = true;
        } else {
            digit = false;
            out.push(c);
        }
    }
    out
}

/// Error lines grouped by mod, with the known-harmless ones counted separately.
pub fn errors(text: &str, ix: &Index) -> (Vec<ErrorGroup>, Vec<NoiseGroup>, Vec<String>) {
    let k = knowledge();
    let mut groups: HashMap<(String, String), (ErrorGroup, std::collections::HashSet<String>)> = HashMap::new();
    let mut noise: Vec<(u32, HashMap<String, u32>)> = k.noise.iter().map(|_| (0, HashMap::new())).collect();
    let mut real_lines = Vec::new();
    for raw in text.lines() {
        let l = parse(raw);
        if l.level != "ERROR" && l.level != "FATAL" {
            continue;
        }
        let lower = l.msg.to_lowercase();
        if let Some(i) = k.noise.iter().position(|n| n.any.iter().any(|a| lower.contains(a.as_str())) && n.all.iter().all(|a| lower.contains(a.as_str()))) {
            let from = namespace_after(l.msg, &lower, &k.noise[i].blame_from)
                .map(|ns| ix.who(&ns).map(|w| w.0).unwrap_or(ns))
                .unwrap_or_else(|| source(ix, l.logger).0);
            noise[i].0 += 1;
            *noise[i].1.entry(from).or_default() += 1;
            continue;
        }
        // Prefer the mod a resource id in the message belongs to ("modid:thing") over the logger.
        let ns = lower.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '/')))
            .filter_map(|w| w.split_once(':').map(|(a, _)| a.to_string()))
            .find(|a| a.len() >= 3 && a != "minecraft" && ix.has(a));
        let (who, file) = match ns.and_then(|a| ix.who(&a)) { Some(w) => w, None => source(ix, l.logger) };
        let e = groups.entry((who.clone(), file.clone())).or_insert_with(|| (ErrorGroup { who, file, count: 0, sample: l.msg.chars().take(300).collect() }, Default::default()));
        e.0.count += 1;
        if e.1.insert(template(l.msg)) && real_lines.len() < 60 {
            real_lines.push(raw.chars().take(400).collect());
        }
    }
    let mut g: Vec<ErrorGroup> = groups.into_values().map(|(g, _)| g).collect();
    g.sort_by(|a, b| b.count.cmp(&a.count));
    let mut n: Vec<NoiseGroup> = noise.into_iter().enumerate().filter(|(_, (c, _))| *c > 0).map(|(i, (count, from))| {
        let mut f: Vec<(String, u32)> = from.into_iter().collect();
        f.sort_by(|a, b| b.1.cmp(&a.1));
        NoiseGroup { label: k.noise[i].label.clone(), count, from: f.into_iter().take(4).map(|(m, _)| m).collect() }
    }).collect();
    n.sort_by(|a, b| b.count.cmp(&a.count));
    (g, n, real_lines)
}

/// Adds the knowledge base, timing and grouped errors to a diagnosis.
pub fn enrich(d: &mut Diagnosis, kind: &str, text: &str, ix: &Index) {
    apply_knowledge(d, kind, text, ix);
    if kind != "log" {
        attach_lines(d, text);
        return;
    }
    let tl = timeline(text, ix);
    let (groups, noise, lines) = errors(text, ix);
    // Replace the generic "N error lines" finding with the grouped view.
    d.findings.retain(|f| !f.title.contains("error line"));
    let real: u32 = groups.iter().map(|g| g.count).sum();
    let harmless: u32 = noise.iter().map(|n| n.count).sum();
    if real > 0 || harmless > 0 {
        let top: Vec<String> = groups.iter().take(3).map(|g| format!("{} ({})", g.who, g.count)).collect();
        d.findings.push(Finding {
            severity: "info".into(),
            title: if real == 0 { format!("{harmless} error lines, all known to be harmless") } else { format!("{real} error line{} worth a look", if real == 1 { "" } else { "s" }) },
            detail: if real == 0 { "Everything the log called an error is a known, harmless message (listed below).".into() }
                else { format!("From {}.{}", top.join(", "), if harmless > 0 { format!(" Another {harmless} are known to be harmless.") } else { String::new() }) },
            fix: if real == 0 { String::new() } else { "These only matter if something in game doesn't work. Each group below says which mod it belongs to.".into() },
            ..Default::default()
        });
    }
    if let Some(m) = tl.by_mod.first().filter(|m| m.seconds >= 10.0 && m.who != "Minecraft") {
        d.findings.push(Finding {
            severity: "warning".into(),
            title: format!("{} held up loading for {:.0} seconds", m.who, m.seconds),
            detail: tl.steps.iter().find(|s| s.who == m.who && !s.note.is_empty()).map(|s| s.note.clone())
                .unwrap_or_else(|| format!("The log went quiet for {:.0} s in total while {} was working.", m.seconds, m.who)),
            fix: format!("If loading feels too slow, check {}'s settings or try the pack without it.", m.who),
            action: if m.file.is_empty() { String::new() } else { "page".into() },
            target: m.file.clone(),
            lines: tl.steps.iter().filter(|s| s.who == m.who).flat_map(|s| [format!("{:.1} s wait:", s.seconds), s.before.clone(), s.after.clone()]).collect(),
        });
    }
    d.excerpt = lines;
    attach_lines(d, text);
    d.timeline = Some(tl);
    d.error_groups = groups;
    d.noise = noise;
    if !d.findings.iter().any(|f| f.severity == "critical") {
        d.headline = match d.findings.iter().find(|f| f.severity == "warning") {
            Some(w) => w.title.clone(),
            None => "The last session looks healthy".into(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ix() -> Index {
        let mut ix = Index::default();
        for (id, name, file) in [("roadweaver", "RoadWeaver", "roadweaver.jar"), ("villagerapi", "Villager API", "villagerapi.jar"), ("nomadictents", "Nomadic Tents", "tents.jar")] {
            ix.ids.push(id.into());
            ix.names.insert(id.into(), (name.into(), file.into()));
        }
        ix
    }

    #[test]
    fn puts_slow_world_load_on_the_right_mod() {
        let log = "[20Oct2026 21:22:00.000] [Render thread/WARN] [ModernFix/]: Game took 91.298 seconds to start\n\
[20Oct2026 21:22:05.000] [Server thread/INFO] [net.minecraft.server.MinecraftServer/]: Starting integrated minecraft server version 1.21.1\n\
[20Oct2026 21:22:06.000] [Server thread/INFO] [villagerapi/]: Validating POI types\n\
[20Oct2026 21:22:12.000] [Server thread/INFO] [villagerapi/]: Registered villager type mappings\n\
[20Oct2026 21:22:27.000] [Server thread/INFO] [Ksyxis/MinecraftServerMixin/]: Ksyxis: Speeding up the world loading...\n\
[20Oct2026 21:22:48.000] [Server thread/INFO] [roadweaver/]: OpenCL not supported, using CPU\n\
[20Oct2026 21:23:04.000] [RW-Initial-1/INFO] [roadweaver/]: Presets loaded\n\
[20Oct2026 21:23:05.000] [Server thread/INFO] [net.minecraft.server.players.PlayerList/]: Dev logged in with entity id 2\n\
[20Oct2026 21:25:05.000] [Render thread/INFO] [net.minecraft.client.Minecraft/]: chat\n";
        let tl = timeline(log, &ix());
        assert!((tl.startup_s - 91.298).abs() < 0.01);
        assert_eq!(tl.world_s, 60.0);
        assert_eq!(tl.by_mod[0].who, "RoadWeaver");
        assert_eq!(tl.by_mod[0].seconds, 38.0);
        assert!(tl.steps.iter().any(|s| s.who == "RoadWeaver" && s.note.contains("OpenCL")));
        // The two minutes in game don't count.
        assert!(tl.steps.iter().all(|s| s.seconds < 100.0));
    }

    #[test]
    fn harmless_errors_are_set_aside_and_blamed() {
        let log = "[20Oct2026 21:22:00.000] [Render thread/ERROR] [net.minecraft.server.ServerAdvancementManager/]: Parsing error loading custom advancement nomadictents:recipes/mega_yurt_layer1: bad\n\
[20Oct2026 21:22:00.000] [Render thread/ERROR] [villagerapi/]: Something broke 12\n\
[20Oct2026 21:22:00.000] [Render thread/ERROR] [villagerapi/]: Something broke 13\n";
        let (g, n, lines) = errors(log, &ix());
        assert_eq!(n[0].count, 1);
        assert_eq!(n[0].from, vec!["Nomadic Tents".to_string()]);
        assert_eq!(g[0].who, "Villager API");
        assert_eq!(g[0].count, 2);
        assert_eq!(lines.len(), 1, "same template counted once");
    }
}
