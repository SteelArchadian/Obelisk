//! Custom test worlds for the AI tools (launch's test_world): a flat or empty world with chosen layers, a world
//! border, an optional platform (in the sky if wanted), and scripted commands at set times. Built as a fresh save
//! in a test copy (level.dat plus a small datapack) before the game starts; the game then opens it.

use serde_json::Value;
use std::io::Write;
use std::path::Path;

/// Saves this module makes start with this, and the previous ones are cleared at the next build.
pub const PREFIX: &str = "obtest-";

// ---------------------------------------------------------------- a small NBT writer

enum Tag {
    Byte(i8),
    Int(i32),
    Long(i64),
    Str(String),
    List(Vec<Tag>),
    Compound(Vec<(String, Tag)>),
}

impl Tag {
    fn id(&self) -> u8 {
        match self { Tag::Byte(_) => 1, Tag::Int(_) => 3, Tag::Long(_) => 4, Tag::Str(_) => 8, Tag::List(_) => 9, Tag::Compound(_) => 10 }
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Tag::Byte(v) => out.push(*v as u8),
            Tag::Int(v) => out.extend(v.to_be_bytes()),
            Tag::Long(v) => out.extend(v.to_be_bytes()),
            Tag::Str(s) => str_bytes(s, out),
            Tag::List(items) => {
                out.push(items.first().map_or(0, Tag::id));
                out.extend((items.len() as i32).to_be_bytes());
                for t in items { t.write(out) }
            }
            Tag::Compound(entries) => {
                for (k, t) in entries {
                    out.push(t.id());
                    str_bytes(k, out);
                    t.write(out);
                }
                out.push(0);
            }
        }
    }
}

fn str_bytes(s: &str, out: &mut Vec<u8>) {
    out.extend((s.len() as u16).to_be_bytes());
    out.extend(s.as_bytes());
}

fn c(entries: Vec<(&str, Tag)>) -> Tag {
    Tag::Compound(entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn st(s: &str) -> Tag {
    Tag::Str(s.to_string())
}

// ---------------------------------------------------------------- the request

struct Spec {
    layers: Vec<(String, i32)>,
    biome: String,
    border: Option<u32>,
    platform: Option<(i32, u32, String)>,
    spawn: Option<(i32, i32, i32)>,
    game_mode: i32,
    setup: Vec<String>,
    events: Vec<(u32, Option<u32>, String)>,
    hold: Option<u32>,
}

fn block_id(v: &str, what: &str) -> Result<String, String> {
    let id = if v.contains(':') { v.to_string() } else { format!("minecraft:{v}") };
    let (ns, path) = id.split_once(':').unwrap();
    let ok = |s: &str, extra: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c) || extra.contains(c));
    if ok(ns, "") && ok(path, "/") && !path.split('/').any(|p| p.is_empty() || p.starts_with('.')) { Ok(id) } else { Err(format!("{what}: \"{v}\" isn't a block or biome id")) }
}

/// One command line for a function file: no slash, one line, a sane length.
fn command(v: &Value, what: &str) -> Result<String, String> {
    let s = v.as_str().ok_or(format!("{what} must be a string"))?.trim();
    let s = s.strip_prefix('/').unwrap_or(s);
    if s.is_empty() || s.len() > 2000 || s.contains(['\n', '\r']) {
        return Err(format!("{what} must be one command line (up to 2000 characters)"));
    }
    Ok(s.to_string())
}

fn int(v: &Value, k: &str, lo: i64, hi: i64) -> Result<Option<i64>, String> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(n) => n.as_i64().filter(|n| (lo..=hi).contains(n)).map(Some).ok_or(format!("{k} must be a whole number from {lo} to {hi}")),
    }
}

fn parse(v: &Value) -> Result<Spec, String> {
    let layers = match v.get("layers") {
        None | Some(Value::Null) => vec![("minecraft:bedrock".into(), 1), ("minecraft:dirt".into(), 2), ("minecraft:grass_block".into(), 1)],
        Some(Value::Array(a)) => a.iter().enumerate().map(|(i, l)| {
            let b = l.get("block").and_then(Value::as_str).ok_or(format!("layers[{i}].block is missing"))?;
            let h = int(l, "height", 1, 384)?.unwrap_or(1) as i32;
            Ok((block_id(b, &format!("layers[{i}]"))?, h))
        }).collect::<Result<Vec<_>, String>>()?,
        _ => return Err("layers must be a list of {block, height}, bottom first".into()),
    };
    if layers.iter().map(|l| l.1).sum::<i32>() > 384 {
        return Err("the layers add up to more than the world's 384 blocks of height".into());
    }
    let biome = block_id(v.get("biome").and_then(Value::as_str).unwrap_or(if layers.is_empty() { "the_void" } else { "plains" }), "biome")?;
    let border = int(v, "border", 16, 60_000_000)?.map(|n| n as u32);
    let platform = match v.get("platform") {
        None | Some(Value::Null) => None,
        Some(p) => Some((int(p, "y", -63, 319)?.unwrap_or(150) as i32, int(p, "size", 1, 1024)?.unwrap_or(32) as u32,
            block_id(p.get("block").and_then(Value::as_str).unwrap_or("stone"), "platform.block")?)),
    };
    let spawn = match v.get("spawn") {
        None | Some(Value::Null) => None,
        Some(s) => Some((int(s, "x", -30_000_000, 30_000_000)?.unwrap_or(0) as i32, int(s, "y", -64, 320)?.ok_or("spawn.y is missing")? as i32,
            int(s, "z", -30_000_000, 30_000_000)?.unwrap_or(0) as i32)),
    };
    let game_mode = match v.get("game_mode").and_then(Value::as_str).unwrap_or("creative") {
        "survival" => 0, "creative" => 1, "adventure" => 2, "spectator" => 3,
        m => return Err(format!("game_mode \"{m}\" isn't survival, creative, adventure or spectator")),
    };
    let setup = match v.get("setup") {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(a)) => a.iter().enumerate().map(|(i, x)| command(x, &format!("setup[{i}]"))).collect::<Result<_, _>>()?,
        _ => return Err("setup must be a list of commands".into()),
    };
    let events = match v.get("events") {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(a)) => a.iter().enumerate().map(|(i, e)| {
            let at = e.get("at").and_then(Value::as_f64).filter(|s| (0.0..=3600.0).contains(s)).ok_or(format!("events[{i}].at must be seconds from 0 to 3600"))?;
            let every = match e.get("every") {
                None | Some(Value::Null) => None,
                Some(x) => Some(x.as_f64().filter(|s| (0.05..=3600.0).contains(s)).ok_or(format!("events[{i}].every must be seconds from 0.05 to 3600"))?),
            };
            Ok(((at * 20.0).round() as u32, every.map(|s| ((s * 20.0).round() as u32).max(1)), command(e.get("command").unwrap_or(&Value::Null), &format!("events[{i}].command"))?))
        }).collect::<Result<_, String>>()?,
        _ => return Err("events must be a list of {at, command, every?}".into()),
    };
    if events.len() + setup.len() > 500 {
        return Err("at most 500 setup commands and events".into());
    }
    let hold = int(v, "hold", 5, 3600)?.map(|n| n as u32);
    Ok(Spec { layers, biome, border, platform, spawn, game_mode, setup, events, hold })
}

/// Minecraft's save format number for each version a test copy may run.
fn data_version(minecraft: &str) -> Result<i32, String> {
    match minecraft {
        "1.21.1" => Ok(3955),
        "1.21" => Ok(3953),
        v => Err(format!("test_world doesn't know Minecraft {v}'s save format yet")),
    }
}

/// How long the game stays in the world before the autobench quits (seconds): the given hold, else long enough
/// for the last event plus a little (repeating events get a minute more), at least 20 s.
fn hold_of(s: &Spec) -> u32 {
    s.hold.unwrap_or_else(|| {
        let last = s.events.iter().map(|e| e.0 / 20 + if e.1.is_some() { 60 } else { 0 }).max().unwrap_or(0);
        (last + 10).max(20)
    })
}

/// Build the save in `<copy>/saves/<PREFIX><stamp>`; returns its folder name and how many seconds the game
/// should stay in it. Earlier test worlds go first.
pub fn build(copy: &Path, minecraft: &str, v: &Value, stamp: u64) -> Result<(String, u32), String> {
    let spec = parse(v)?;
    let dv = data_version(minecraft)?;
    let saves = copy.join("saves");
    for e in std::fs::read_dir(&saves).into_iter().flatten().flatten() {
        if e.file_name().to_string_lossy().starts_with(PREFIX) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    let name = format!("{PREFIX}{stamp}");
    let dir = saves.join(&name);
    let pack = dir.join("datapacks").join("obelisk_test");
    std::fs::create_dir_all(&pack).map_err(|e| e.to_string())?;
    level_dat(&dir, &name, &spec, dv)?;
    datapack(&pack, &spec)?;
    Ok((name, hold_of(&spec)))
}

/// Where the player starts: the given spawn, else on top of the platform, else on top of the layers.
fn spawn_of(s: &Spec) -> (i32, i32, i32) {
    s.spawn.unwrap_or_else(|| match &s.platform {
        Some((y, _, _)) => (0, y + 1, 0),
        None => (0, -64 + s.layers.iter().map(|l| l.1).sum::<i32>().max(1), 0),
    })
}

fn level_dat(dir: &Path, name: &str, s: &Spec, dv: i32) -> Result<(), String> {
    let layers = if s.layers.is_empty() { vec![("minecraft:air".to_string(), 1)] } else { s.layers.clone() };
    let flat = c(vec![
        ("biome", st(&s.biome)),
        ("features", Tag::Byte(0)),
        ("lakes", Tag::Byte(0)),
        ("layers", Tag::List(layers.iter().map(|(b, h)| c(vec![("block", st(b)), ("height", Tag::Int(*h))])).collect())),
        ("structure_overrides", Tag::List(vec![])),
    ]);
    let dims = c(vec![
        ("minecraft:overworld", c(vec![("type", st("minecraft:overworld")),
            ("generator", c(vec![("type", st("minecraft:flat")), ("settings", flat)]))])),
        ("minecraft:the_nether", c(vec![("type", st("minecraft:the_nether")),
            ("generator", c(vec![("type", st("minecraft:noise")), ("settings", st("minecraft:nether")),
                ("biome_source", c(vec![("type", st("minecraft:multi_noise")), ("preset", st("minecraft:nether"))]))]))])),
        ("minecraft:the_end", c(vec![("type", st("minecraft:the_end")),
            ("generator", c(vec![("type", st("minecraft:noise")), ("settings", st("minecraft:end")),
                ("biome_source", c(vec![("type", st("minecraft:the_end"))]))]))])),
    ]);
    let (x, y, z) = spawn_of(s);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let data = c(vec![
        ("DataVersion", Tag::Int(dv)),
        ("version", Tag::Int(19133)),
        ("Version", c(vec![("Id", Tag::Int(dv)), ("Name", st(if dv == 3955 { "1.21.1" } else { "1.21" })), ("Series", st("main")), ("Snapshot", Tag::Byte(0))])),
        ("LevelName", st(name)),
        ("GameType", Tag::Int(s.game_mode)),
        ("Difficulty", Tag::Byte(2)),
        ("hardcore", Tag::Byte(0)),
        ("allowCommands", Tag::Byte(1)),
        ("initialized", Tag::Byte(1)),
        ("SpawnX", Tag::Int(x)), ("SpawnY", Tag::Int(y)), ("SpawnZ", Tag::Int(z)), ("SpawnAngle", Tag::Int(0)),
        ("Time", Tag::Long(0)), ("DayTime", Tag::Long(6000)), ("LastPlayed", Tag::Long(now)),
        ("raining", Tag::Byte(0)), ("thundering", Tag::Byte(0)), ("clearWeatherTime", Tag::Int(0)),
        ("DataPacks", c(vec![("Enabled", Tag::List(vec![st("vanilla"), st("file/obelisk_test")])), ("Disabled", Tag::List(vec![]))])),
        ("enabled_features", Tag::List(vec![st("minecraft:vanilla")])),
        ("DragonFight", c(vec![("NeedsStateScanning", Tag::Byte(1)), ("DragonKilled", Tag::Byte(0)), ("PreviouslyKilled", Tag::Byte(0))])),
        ("WorldGenSettings", c(vec![("seed", Tag::Long(0)), ("generate_features", Tag::Byte(0)), ("bonus_chest", Tag::Byte(0)), ("dimensions", dims)])),
    ]);
    let root = c(vec![("Data", data)]);
    let mut raw = vec![10u8];
    str_bytes("", &mut raw);
    root.write(&mut raw);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&raw).map_err(|e| e.to_string())?;
    let bytes = gz.finish().map_err(|e| e.to_string())?;
    std::fs::write(dir.join("level.dat"), bytes).map_err(|e| e.to_string())
}

/// pack.mcmeta, the load/tick tags and the functions: setup once, the platform once its chunks are loaded,
/// then each event at its tick (counted from when a player is first in the world).
fn datapack(pack: &Path, s: &Spec) -> Result<(), String> {
    let w = |rel: &str, text: String| -> Result<(), String> {
        let p = pack.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(p, text).map_err(|e| e.to_string())
    };
    w("pack.mcmeta", r#"{"pack":{"pack_format":48,"description":"Obelisk test world"}}"#.into())?;
    w("data/minecraft/tags/function/load.json", r#"{"values":["obelisk_test:load"]}"#.into())?;
    w("data/minecraft/tags/function/tick.json", r#"{"values":["obelisk_test:tick"]}"#.into())?;
    let (x, y, z) = spawn_of(s);
    w("data/obelisk_test/function/load.mcfunction", "scoreboard objectives add obelisk_t dummy\n\
        execute unless score #setup obelisk_t matches 1.. run function obelisk_test:setup\n".into())?;

    let mut setup = vec!["scoreboard players set #setup obelisk_t 1".to_string(), "scoreboard players set #t obelisk_t -1".to_string(),
        format!("setworldspawn {x} {y} {z}")];
    if let Some(b) = s.border {
        setup.push(format!("worldborder center {x} {z}"));
        setup.push(format!("worldborder set {b}"));
    }
    let mut build = vec!["scoreboard players set #setup obelisk_t 2".to_string()];
    let mut check = String::new();
    if let Some((py, size, block)) = &s.platform {
        let lo = -((*size as i32) / 2);
        let hi = lo + *size as i32 - 1;
        setup.push(format!("forceload add {} {} {} {}", x + lo, z + lo, x + hi, z + hi));
        // fill takes up to 32768 blocks at a time: 128 x 128 tiles. An invisible barrier layer underneath holds up
        // blocks that would otherwise fall (sand, gravel, concrete powder).
        let mut a = lo;
        while a <= hi {
            let mut b = lo;
            while b <= hi {
                let (x1, z1, x2, z2) = (x + a, z + b, x + (a + 127).min(hi), z + (b + 127).min(hi));
                if *py > -64 { build.push(format!("fill {x1} {} {z1} {x2} {} {z2} minecraft:barrier", py - 1, py - 1)); }
                build.push(format!("fill {x1} {py} {z1} {x2} {py} {z2} {block}"));
                b += 128;
            }
            a += 128;
        }
        build.push(format!("forceload remove {} {} {} {}", x + lo, z + lo, x + hi, z + hi));
        check = format!("if loaded {} {py} {} if loaded {} {py} {} if loaded {x} {py} {z} ", x + lo, z + lo, x + hi, z + hi);
    }
    setup.extend(s.setup.iter().cloned());
    w("data/obelisk_test/function/setup.mcfunction", setup.join("\n") + "\n")?;
    w("data/obelisk_test/function/build.mcfunction", build.join("\n") + "\n")?;

    let mut tick = vec![format!("execute if score #setup obelisk_t matches 1 {check}run function obelisk_test:build")];
    // The clock starts when a player is first in: they land at the start, in the chosen game mode.
    tick.push("execute if score #t obelisk_t matches -1 if entity @a run function obelisk_test:arrive".into());
    tick.push("execute if score #t obelisk_t matches 0.. run scoreboard players add #t obelisk_t 1".into());
    for (i, (at, every, _)) in s.events.iter().enumerate() {
        let at = at + 1;
        match every {
            None => tick.push(format!("execute if score #t obelisk_t matches {at} run function obelisk_test:event_{i}")),
            Some(n) => {
                // Repeats: at, at+n, at+2n, ... (the remainder of (t - at) / n is 0 from `at` on).
                tick.push(format!("execute if score #t obelisk_t matches {at}.. run scoreboard players operation #r obelisk_t = #t obelisk_t"));
                tick.push(format!("execute if score #t obelisk_t matches {at}.. run scoreboard players remove #r obelisk_t {at}"));
                tick.push(format!("scoreboard players set #n obelisk_t {n}"));
                tick.push(format!("execute if score #t obelisk_t matches {at}.. run scoreboard players operation #r obelisk_t %= #n obelisk_t"));
                tick.push(format!("execute if score #t obelisk_t matches {at}.. if score #r obelisk_t matches 0 run function obelisk_test:event_{i}"));
            }
        }
    }
    w("data/obelisk_test/function/tick.mcfunction", tick.join("\n") + "\n")?;
    let mode = ["survival", "creative", "adventure", "spectator"][s.game_mode as usize];
    w("data/obelisk_test/function/arrive.mcfunction", format!("scoreboard players set #t obelisk_t 0\n\
        tp @a {x}.5 {y} {z}.5\ngamemode {mode} @a\n"))?;
    for (i, (_, _, cmd)) in s.events.iter().enumerate() {
        w(&format!("data/obelisk_test/function/event_{i}.mcfunction"), format!("{cmd}\n"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one into a real copy, for a check in the game: OBELISK_TW_COPY=<game dir> OBELISK_TW_SPEC=<json>.
    #[test]
    #[ignore]
    fn build_into_a_copy() {
        let copy = std::env::var("OBELISK_TW_COPY").unwrap();
        let spec: Value = serde_json::from_str(&std::env::var("OBELISK_TW_SPEC").unwrap()).unwrap();
        println!("SAVE {:?}", build(Path::new(&copy), "1.21.1", &spec, 1).unwrap());
    }

    #[test]
    fn builds_a_sky_platform_world() {
        let t = tempfile::tempdir().unwrap();
        let v = serde_json::json!({ "layers": [], "border": 64, "platform": { "y": 120, "size": 200, "block": "sand" },
            "events": [{ "at": 5, "command": "/summon zombie 3 121 3" }, { "at": 10, "every": 2.5, "command": "say hi" }] });
        let (name, hold) = build(t.path(), "1.21.1", &v, 1).unwrap();
        assert_eq!(hold, 80);
        let d = t.path().join("saves").join(&name);
        assert!(d.join("level.dat").is_file());
        let f = |p: &str| std::fs::read_to_string(d.join("datapacks/obelisk_test/data/obelisk_test/function").join(p)).unwrap();
        assert_eq!(f("build.mcfunction").lines().filter(|l| l.starts_with("fill")).count(), 8);
        assert!(f("build.mcfunction").contains("fill -100 119 -100 27 119 27 minecraft:barrier
fill -100 120 -100 27 120 27 minecraft:sand"));
        assert!(f("setup.mcfunction").contains("worldborder set 64"));
        assert_eq!(f("event_0.mcfunction"), "summon zombie 3 121 3\n");
        assert!(f("tick.mcfunction").contains("matches 101 run function obelisk_test:event_0"));
        assert!(build(t.path(), "1.21.1", &serde_json::json!({ "events": [{ "at": 1, "command": "say a\nop me" }] }), 2).is_err());
        assert!(build(t.path(), "1.21.1", &serde_json::json!({ "layers": [{ "block": "../x" }] }), 3).is_err());
        // A new test world clears the earlier one.
        build(t.path(), "1.21.1", &serde_json::json!({}), 4).unwrap();
        assert!(!d.exists());
    }
}
