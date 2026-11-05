//! Talking to a running test game through the harness probe (testharness, -Dtestharness.probe=true): short JSON
//! answers instead of logs and screenshots. The game writes its port and token to <copy>/.obelisk/probe.json.
//! A scenario can run in the background: its progress is kept per job (game_result), and game_stop hands the game
//! back to the chat between steps.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Per job: the scenario's progress so far, and whether the chat asked to take over.
#[derive(Default, Clone)]
pub(crate) struct Progress {
    pub phase: String,
    pub step: usize,
    pub steps: usize,
    pub results: Vec<Value>,
    pub stop: bool,
    pub result: Option<Value>,
}

fn table() -> &'static Mutex<HashMap<u32, Progress>> {
    static T: OnceLock<Mutex<HashMap<u32, Progress>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

pub(crate) fn progress(job: u32, f: impl FnOnce(&mut Progress)) {
    f(table().lock().unwrap().entry(job).or_default())
}

pub(crate) fn progress_of(job: u32) -> Option<Progress> {
    table().lock().unwrap().get(&job).cloned()
}

fn stopped(job: u32) -> bool {
    table().lock().unwrap().get(&job).is_some_and(|p| p.stop)
}

/// One request to the game in `dir` (started at unix second `started`, so an old run's probe file isn't used).
pub(crate) fn ask(dir: &Path, started: u64, mut req: Value) -> Result<Value, String> {
    let file = dir.join(".obelisk").join("probe.json");
    let fresh = std::fs::metadata(&file).and_then(|m| m.modified()).ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).is_some_and(|t| t.as_secs() + 5 >= started);
    if !fresh {
        return Err("the game isn't answering yet (no probe from this run; it starts once mods have loaded)".into());
    }
    let info: Value = serde_json::from_slice(&std::fs::read(&file).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let port = info["port"].as_u64().filter(|p| *p > 0 && *p < 65536).ok_or("probe.json has no port")? as u16;
    req["token"] = info["token"].clone();
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(|e| format!("the game's probe didn't answer: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(45))).ok();
    s.write_all(format!("{req}\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).map_err(|e| format!("the game's probe didn't answer: {e}"))?;
    let v: Value = serde_json::from_str(&line).map_err(|e| format!("probe reply: {e}"))?;
    match v.get("error").and_then(|e| e.as_str()) {
        Some(e) => Err(e.to_string()),
        None => Ok(v),
    }
}

/// A query without the empty error list, to keep answers short.
fn query(dir: &Path, started: u64) -> Result<Value, String> {
    let mut q = ask(dir, started, json!({ "op": "query" }))?;
    if q["errorCount"] == 0 { if let Some(o) = q.as_object_mut() { o.remove("errors"); } }
    Ok(q)
}

pub(crate) fn command(cmd: &str, args: &Value, job: u32, dir: &Path, started: u64) -> Result<Value, String> {
    match cmd {
        "game-query" => query(dir, started),
        "game-cmd" => {
            let c = args.get("command").and_then(|c| c.as_str()).filter(|c| !c.trim().is_empty()).ok_or("missing \"command\"")?;
            ask(dir, started, json!({ "op": "cmd", "command": c }))
        }
        "game-scenario" => scenario(args, job, dir, started),
        "game-quit" => ask(dir, started, json!({ "op": "quit" })),
        _ => Err(format!("unknown command {cmd}")),
    }
}

fn nums(v: &Value) -> Option<Vec<f64>> {
    v.as_array()?.iter().map(|n| n.as_f64()).collect()
}

/// Run the steps in order and return one compact result; stops at the first failed step or assert, or when the
/// chat takes over (game_stop).
pub(crate) fn scenario(args: &Value, job: u32, dir: &Path, started: u64) -> Result<Value, String> {
    let steps = args.get("steps").and_then(|s| s.as_array()).filter(|s| !s.is_empty()).ok_or("missing \"steps\" (a list)")?;
    let limit = args.get("timeout").and_then(|t| t.as_u64()).unwrap_or(300).clamp(10, 900);
    let end = Instant::now() + Duration::from_secs(limit);
    let pause = move |secs: f64| -> Result<(), String> {
        let d = Duration::from_secs_f64(secs.max(0.0));
        if stopped(job) { return Err("stopped: the chat took over".into()) }
        if Instant::now() + d > end { return Err(format!("the scenario would pass its {limit} s timeout")) }
        std::thread::sleep(d);
        Ok(())
    };
    progress(job, |p| { p.steps = steps.len(); p.step = 0; p.results.clear(); p.phase = "steps".into(); });
    let g = Game { dir, started, pause: &pause };
    let mut last = Value::Null;
    // Structures found by locate steps, nearest first: (name, x, z, blocks away).
    let mut found: Vec<(String, f64, f64, f64)> = Vec::new();
    let mut results = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        progress(job, |p| p.step = i);
        let r = if stopped(job) { Err("stopped: the chat took over".to_string()) } else { g.step(step, &last, &mut found) };
        match r {
            Ok(v) => {
                if step.get("assert").is_none() { last = v.clone() }
                let row = json!({ "step": i, "do": step.as_object().and_then(|o| o.keys().next().cloned()), "result": v });
                progress(job, |p| p.results.push(row.clone()));
                results.push(row);
            }
            Err(e) => return Ok(json!({ "ok": false, "failedStep": i, "error": e, "results": results })),
        }
    }
    Ok(json!({ "ok": true, "results": results }))
}

/// The ready-made tests behind game_test's preset: "basic" (does it run: visit 4 nearby structures, 10 s each)
/// and "stress" (tests: chests, mobs, hop, flyover; default all).
pub(crate) fn preset(name: &str, tests: &[String]) -> Result<Vec<Value>, String> {
    let locate = json!({ "locate": ["#minecraft:village", "minecraft:pillager_outpost", "#minecraft:ruined_portal", "minecraft:desert_pyramid",
        "minecraft:jungle_pyramid", "minecraft:igloo", "minecraft:shipwreck", "minecraft:swamp_hut", "minecraft:trail_ruins"] });
    let mut steps = vec![json!({ "cmd": "gamemode spectator" })];
    match name {
        "basic" => {
            steps.push(locate);
            steps.push(json!({ "visit": { "count": 4, "wait": 10 } }));
        }
        "stress" => {
            let all = ["chests", "mobs", "hop", "flyover"];
            let pick: Vec<&str> = if tests.is_empty() { all.to_vec() } else { tests.iter().map(String::as_str).collect() };
            if let Some(t) = pick.iter().find(|t| !all.contains(t)) { return Err(format!("unknown stress test {t}: use {}", all.join(", "))) }
            if pick.contains(&"hop") { steps.push(locate) }
            for t in pick {
                steps.push(match t {
                    "chests" => json!({ "chests": { "size": 16, "height": 4, "seconds": 15 } }),
                    "mobs" => json!({ "mobs": { "count": 100, "seconds": 15 } }),
                    "hop" => json!({ "hop": { "every": 2, "rounds": 2 } }),
                    _ => json!({ "flyover": { "seconds": 40, "speed": 20, "low": 32, "high": 60 } }),
                });
            }
        }
        _ => return Err(format!("unknown preset {name}: use basic or stress")),
    }
    steps.push(json!({ "query": true }));
    Ok(steps)
}

struct Game<'a> {
    dir: &'a Path,
    started: u64,
    pause: &'a dyn Fn(f64) -> Result<(), String>,
}

impl Game<'_> {
    fn ask(&self, req: Value) -> Result<Value, String> { ask(self.dir, self.started, req) }
    fn cmd(&self, c: String) -> Result<Value, String> { self.ask(json!({ "op": "cmd", "command": c })) }
    fn query(&self) -> Result<Value, String> { query(self.dir, self.started) }
    fn ground(&self, x: f64, z: f64) -> Result<f64, String> {
        self.ask(json!({ "op": "ground", "x": x.floor() as i64, "z": z.floor() as i64 }))?["y"].as_f64().ok_or("no ground height".into())
    }
    fn pos(&self) -> Result<(f64, f64, f64), String> {
        let q = self.query()?;
        let p = &q["player"];
        Ok((p["x"].as_f64().ok_or("no player in the world")?, p["y"].as_f64().unwrap_or(100.0), p["z"].as_f64().unwrap_or(0.0)))
    }

    fn step(&self, step: &Value, last: &Value, found: &mut Vec<(String, f64, f64, f64)>) -> Result<Value, String> {
        let n = |v: &Value, k: &str, d: f64| v[k].as_f64().unwrap_or(d);
        if let Some(c) = step.get("cmd").and_then(|c| c.as_str()) {
            return self.cmd(c.to_string());
        }
        if let Some(p) = step.get("tp").and_then(nums).filter(|p| p.len() == 3 || p.len() == 5) {
            let look = if p.len() == 5 { format!(" {} {}", p[3], p[4]) } else { String::new() };
            let r = self.cmd(format!("tp @s {} {} {}{look}", p[0], p[1], p[2]))?;
            (self.pause)(0.25)?; // the client sees the new position a tick later
            return Ok(r);
        }
        if let Some(w) = step.get("wait").and_then(|w| w.as_f64()) {
            (self.pause)(w)?;
            return Ok(json!({ "waited": w }));
        }
        if let Some(max) = step.get("wait_chunks").and_then(|w| w.as_f64()) {
            let t0 = Instant::now();
            loop {
                let q = self.query()?;
                if q["renderReady"] == true { return Ok(json!({ "ready": true, "seconds": t0.elapsed().as_secs_f64().round() })) }
                if t0.elapsed().as_secs_f64() >= max { return Ok(json!({ "ready": false, "seconds": max, "chunksLoaded": q["chunksLoaded"] })) }
                (self.pause)(1.0)?;
            }
        }
        if let Some(f) = step.get("fly") {
            let path: Vec<Vec<f64>> = f.get("path").and_then(|p| p.as_array()).map(|p| p.iter().filter_map(nums).filter(|v| v.len() == 3).collect())
                .filter(|p: &Vec<Vec<f64>>| p.len() >= 2).ok_or("fly needs path: two or more [x,y,z] points")?;
            let secs = n(f, "seconds", 20.0).max(1.0);
            let ticks = (secs * 4.0) as usize;
            let mut i = 0;
            let m = self.measure(secs, |_| {
                // four hops a second along the path
                for _ in 0..4 {
                    if i > ticks { break }
                    let at = i as f64 / ticks as f64 * (path.len() - 1) as f64;
                    let k = (at.floor() as usize).min(path.len() - 2);
                    let frac = at - k as f64;
                    let p: Vec<f64> = (0..3).map(|d| path[k][d] + (path[k + 1][d] - path[k][d]) * frac).collect();
                    self.cmd(format!("tp @s {:.2} {:.2} {:.2}", p[0], p[1], p[2]))?;
                    (self.pause)(0.25)?;
                    i += 1;
                }
                Ok(())
            })?;
            return Ok(m);
        }
        if let Some(secs) = step.get("sample").and_then(|s| s.as_f64()) {
            return self.measure(secs, |_| (self.pause)(1.0));
        }
        if step.get("query").is_some() {
            return self.query();
        }
        if let Some(a) = step.get("assert") {
            let field = a.get("field").and_then(|f| f.as_str()).ok_or("assert needs field")?;
            let v = last.pointer(&format!("/{}", field.replace('.', "/"))).cloned().unwrap_or(Value::Null);
            let ok = match (v.as_f64(), a.get("min").and_then(|m| m.as_f64()), a.get("max").and_then(|m| m.as_f64()), a.get("equals")) {
                (_, _, _, Some(e)) => &v == e,
                (Some(x), lo, hi, None) => lo.is_none_or(|lo| x >= lo) && hi.is_none_or(|hi| x <= hi),
                _ => false,
            };
            if !ok { return Err(format!("assert {field} failed: it is {v}, wanted {}", json!(a))) }
            return Ok(json!({ "assert": field, "value": v }));
        }
        if let Some(list) = step.get("locate").and_then(|l| l.as_array()) {
            let names: Vec<&str> = list.iter().filter_map(|n| n.as_str()).collect();
            return self.locate(&names, found);
        }
        if let Some(v) = step.get("visit") {
            return self.visit(found, n(v, "count", 4.0) as usize, n(v, "wait", 10.0), n(v, "above", 20.0));
        }
        if let Some(v) = step.get("hop") {
            return self.hop(found, n(v, "every", 2.0).max(0.5), n(v, "rounds", 2.0) as usize);
        }
        if let Some(v) = step.get("flyover") {
            return self.flyover(n(v, "seconds", 60.0), n(v, "speed", 20.0), n(v, "low", 32.0), n(v, "high", 60.0));
        }
        if let Some(v) = step.get("mobs") {
            return self.mobs((n(v, "count", 100.0) as usize).min(500), n(v, "seconds", 15.0));
        }
        if let Some(v) = step.get("chests") {
            return self.chests((n(v, "size", 16.0) as i64).clamp(2, 32), (n(v, "height", 4.0) as i64).clamp(1, 16), n(v, "seconds", 15.0));
        }
        if let Some(v) = step.get("summon") {
            return self.summon(v);
        }
        if let Some(v) = step.get("punch") {
            return self.punch(n(v, "times", 1.0).clamp(1.0, 50.0) as usize, n(v, "every", 1.0).max(0.1),
                &v.get("shots").and_then(nums).unwrap_or_default(), v["label"].as_str().unwrap_or("punch"));
        }
        if let Some(max) = step.get("settle").and_then(|w| w.as_f64()) {
            // Until the player stays put for 2 s: a world can still move them to its spawn after they arrive.
            let t0 = Instant::now();
            let (mut last, mut still) = (None, Instant::now());
            loop {
                let q = self.query()?;
                let p = &q["player"];
                let now = (p["x"].clone(), p["y"].clone(), p["z"].clone(), p["yaw"].clone());
                if last.as_ref() != Some(&now) { last = Some(now); still = Instant::now() }
                if still.elapsed().as_secs_f64() >= 2.0 { return Ok(json!({ "settled": true, "seconds": t0.elapsed().as_secs_f64().round(), "player": q["player"] })) }
                if t0.elapsed().as_secs_f64() >= max { return Ok(json!({ "settled": false, "seconds": max, "player": q["player"] })) }
                (self.pause)(0.5)?;
            }
        }
        if let Some(v) = step.get("record") {
            let label: String = v["label"].as_str().unwrap_or("rec").chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).take(40).collect();
            let req = json!({ "op": "record", "seconds": v["seconds"].as_f64().unwrap_or(10.0).clamp(1.0, 30.0), "name": format!("{}-{label}", self.started) });
            let during: Vec<Value> = v["during"].as_array().cloned().unwrap_or_default();
            if during.is_empty() { return self.ask(req) }
            // Recording while other steps run (fly a path, walk through a portal): artifacts that only show in motion.
            let (dir, started) = (self.dir, self.started);
            return std::thread::scope(|sc| {
                let rec = sc.spawn(move || ask(dir, started, req));
                (self.pause)(0.3)?; // the recording starts on the next frame
                let mut done = Vec::new();
                for d in &during {
                    match self.step(d, &Value::Null, found) {
                        Ok(r) => done.push(r),
                        Err(e) => { done.push(json!({ "error": e })); break }
                    }
                }
                let mut out = rec.join().map_err(|_| "the recording thread failed".to_string())??;
                out["during"] = json!(done);
                Ok(out)
            });
        }
        if let Some(v) = step.get("shot") {
            return self.shot(v.as_str().or_else(|| v["name"].as_str()).unwrap_or("shot"));
        }
        Err(format!("unknown step {step}: use cmd, tp, wait, wait_chunks, fly, sample, query, assert, locate, visit, hop, flyover, mobs, chests, summon, punch, shot, record or settle"))
    }

    /// A screenshot named for this run and the label, so the same label can be compared across runs.
    fn shot(&self, label: &str) -> Result<Value, String> {
        let label: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).take(40).collect();
        self.ask(json!({ "op": "shot", "name": format!("obt-{}-{label}", self.started) }))
    }

    /// A mob `distance` blocks in front of the player at its feet height, facing it; no AI by default (it stands still).
    fn summon(&self, v: &Value) -> Result<Value, String> {
        let mob = v.as_str().or_else(|| v["mob"].as_str()).ok_or("summon needs a mob, e.g. {\"summon\":{\"mob\":\"zombie\"}}")?;
        if !mob.chars().all(|c| c.is_ascii_alphanumeric() || "_:.-/".contains(c)) { return Err("summon: a mob id like zombie or minecraft:warden".into()) }
        let mob = if mob.contains(':') { mob.to_string() } else { format!("minecraft:{mob}") };
        let dist = v["distance"].as_f64().unwrap_or(3.0).clamp(1.0, 32.0);
        let q = self.query()?;
        let p = &q["player"];
        let (x, y, z) = (p["x"].as_f64().ok_or("no player in the world")?, p["y"].as_f64().unwrap_or(0.0), p["z"].as_f64().unwrap_or(0.0));
        let yaw = p["yaw"].as_f64().unwrap_or(0.0).to_radians();
        let (mx, mz) = (x - yaw.sin() * dist, z + yaw.cos() * dist);
        let mut nbt = vec!["PersistenceRequired:1b".to_string()];
        if !v["ai"].as_bool().unwrap_or(false) { nbt.push("NoAI:1b".into()) }
        // Punches don't push it out of frame unless asked.
        if !v["knockback"].as_bool().unwrap_or(false) { nbt.push("attributes:[{id:\"minecraft:generic.knockback_resistance\",base:1.0d}]".into()) }
        if let Some(extra) = v["nbt"].as_str().map(|t| t.trim().trim_start_matches('{').trim_end_matches('}')).filter(|t| !t.is_empty()) { nbt.push(extra.into()) }
        let out = self.cmd(format!("summon {mob} {mx:.2} {y:.2} {mz:.2} {{{}}}", nbt.join(",")))?;
        (self.pause)(0.25)?;
        // Face its eyes so the punch lands and the shot shows it.
        self.cmd(format!("tp @s {x:.2} {y:.2} {z:.2} facing entity @e[type={mob},limit=1,sort=nearest] eyes"))?;
        (self.pause)(0.25)?;
        Ok(json!({ "summoned": mob, "at": [mx, y, mz], "output": out["output"] }))
    }

    /// `times` punches `every` seconds apart; after each, screenshots at the given offsets (seconds after that punch).
    fn punch(&self, times: usize, every: f64, shots: &[f64], label: &str) -> Result<Value, String> {
        let mut plan: Vec<(f64, Option<(usize, f64)>)> = Vec::new(); // (when, None = punch | Some(punch, offset) = shot)
        for i in 0..times {
            let t = i as f64 * every;
            plan.push((t, None));
            for &s in shots.iter().filter(|s| **s >= 0.0 && **s <= 30.0) { plan.push((t + s, Some((i, s)))) }
        }
        plan.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.is_some().cmp(&b.1.is_some())));
        let t0 = Instant::now();
        let (mut hits, mut files) = (Vec::new(), Vec::new());
        for (at, what) in plan {
            let wait = at - t0.elapsed().as_secs_f64();
            if wait > 0.0 { (self.pause)(wait)? }
            match what {
                None => hits.push(self.ask(json!({ "op": "attack" }))?),
                Some((i, s)) => {
                    let f = self.shot(&format!("{label}{}-{}ms", i + 1, (s * 1000.0).round()))?;
                    files.push(json!({ "punch": i + 1, "after": s, "file": f["file"], "late_ms": ((t0.elapsed().as_secs_f64() - at) * 1000.0).round() }));
                }
            }
        }
        Ok(json!({ "hits": hits, "shots": files }))
    }

    /// Frame and tick stats over `secs`, `during` running once per loop (it should take about a second); readyS =
    /// the first second every visible chunk was built.
    fn measure(&self, secs: f64, mut during: impl FnMut(f64) -> Result<(), String>) -> Result<Value, String> {
        self.ask(json!({ "op": "frames" }))?;
        let t0 = Instant::now();
        let (mut mspt, mut ready, mut heap, mut behind) = (Vec::new(), None, 0.0_f64, 0u32);
        while t0.elapsed().as_secs_f64() < secs {
            during(t0.elapsed().as_secs_f64())?;
            let q = self.query()?;
            if q["busy"] == true { return Ok(json!({ "busy": true, "at": t0.elapsed().as_secs_f64().round() })) }
            if let Some(m) = q["mspt"].as_f64() { mspt.push(m) }
            heap = heap.max(q["heapUsedMB"].as_f64().unwrap_or(0.0));
            if q["renderReady"] == true { ready.get_or_insert(t0.elapsed().as_secs_f64().round()); } else { behind += 1 }
        }
        let mut f = self.ask(json!({ "op": "frames" }))?;
        if !mspt.is_empty() {
            f["mspt"] = json!((mspt.iter().sum::<f64>() / mspt.len() as f64 * 10.0).round() / 10.0);
            f["msptMax"] = json!(mspt.iter().cloned().fold(0.0, f64::max));
        }
        f["readyS"] = json!(ready);
        // Checks where chunks in view weren't built yet: chunk loading falling behind.
        f["checksBehind"] = json!(behind);
        f["heapPeakMB"] = json!(heap);
        Ok(f)
    }

    fn locate(&self, names: &[&str], found: &mut Vec<(String, f64, f64, f64)>) -> Result<Value, String> {
        // "The nearest minecraft:village_plains is at [-240, ~, 112] (268 blocks away)"
        let parse = |s: &str| -> Option<(f64, f64, f64)> {
            let a = s.find('[')?;
            let b = s[a..].find(']')? + a;
            let parts: Vec<&str> = s[a + 1..b].split(',').map(str::trim).collect();
            let dist = s[b..].split(|c: char| !c.is_ascii_digit()).find(|t| !t.is_empty())?.parse().ok()?;
            Some((parts.first()?.parse().ok()?, parts.get(2)?.parse().ok()?, dist))
        };
        let mut missing = 0;
        for name in names {
            let out = self.cmd(format!("locate structure {name}")).ok();
            match out.as_ref().and_then(|o| o["output"].as_array()).and_then(|o| o.iter().filter_map(|l| l.as_str()).find_map(parse)) {
                Some((x, z, d)) => found.push((name.trim_start_matches('#').trim_start_matches("minecraft:").to_string(), x, z, d)),
                None => missing += 1,
            }
        }
        found.sort_by(|a, b| a.3.total_cmp(&b.3));
        Ok(json!({ "found": found.iter().map(|f| json!({ "name": f.0, "away": f.3 })).collect::<Vec<_>>(), "notFound": missing }))
    }

    fn visit(&self, found: &[(String, f64, f64, f64)], count: usize, wait: f64, above: f64) -> Result<Value, String> {
        if found.is_empty() { return Err("no structures found nearby to visit".into()) }
        let mut out = Vec::new();
        for (name, x, z, away) in found.iter().take(count) {
            let y = self.ground(*x, *z)? + above;
            self.cmd(format!("tp @s {x} {y} {z} 0 30"))?;
            let mut m = self.measure(wait, |_| (self.pause)(1.0))?;
            m["structure"] = json!(name);
            m["away"] = json!(away);
            out.push(m);
        }
        Ok(json!(out))
    }

    fn hop(&self, found: &[(String, f64, f64, f64)], every: f64, rounds: usize) -> Result<Value, String> {
        if found.is_empty() { return Err("no structures found nearby to hop between".into()) }
        let stops: Vec<_> = (0..rounds.max(1)).flat_map(|_| found.iter()).collect();
        let mut next = 0usize;
        let m = self.measure(stops.len() as f64 * every, |t| {
            if (t / every) as usize >= next && next < stops.len() {
                let (_, x, z, _) = stops[next];
                self.cmd(format!("tp @s {x} 180 {z}"))?;
                next += 1;
            }
            (self.pause)(every.min(1.0))
        })?;
        Ok(json!({ "stops": stops.len(), "stats": m }))
    }

    /// Straight east, holding low..high blocks above whatever is below (ground, water, leaves): chunk loading and
    /// generation keeping up with flight.
    fn flyover(&self, secs: f64, speed: f64, low: f64, high: f64) -> Result<Value, String> {
        let (mut x, mut y, z) = self.pos()?;
        let start = x;
        let (mut lowest, mut highest) = (f64::MAX, 0.0_f64);
        let mut m = self.measure(secs, |_| {
            for _ in 0..4 {
                x += speed / 4.0;
                let ground = self.ground(x + speed, z)?.max(self.ground(x, z)?);
                if y - ground < low || y - ground > high { y = ground + (low + high) / 2.0 }
                lowest = lowest.min(y - ground);
                highest = highest.max(y - ground);
                self.cmd(format!("tp @s {x:.1} {y:.1} {z:.1} -90 20"))?;
                (self.pause)(0.25)?;
            }
            Ok(())
        })?;
        m["blocks"] = json!((x - start).round());
        m["aboveGround"] = json!([lowest.round(), highest.round()]);
        Ok(m)
    }

    fn mobs(&self, count: usize, secs: f64) -> Result<Value, String> {
        const KINDS: &[&str] = &["zombie", "skeleton", "spider", "creeper", "cow", "sheep", "pig", "chicken", "villager", "wolf",
            "horse", "rabbit", "slime", "witch", "enderman", "llama", "goat", "fox"];
        let (x, _, z) = self.pos()?;
        let ground = self.ground(x, z)?;
        let mut seed = self.started | 1;
        let mut summoned = 0;
        for i in 0..count {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let kind = KINDS[(seed >> 33) as usize % KINDS.len()];
            let (dx, dz) = ((i % 12) as f64 * 1.5 - 9.0, (i / 12) as f64 * 1.5 - 9.0);
            if self.cmd(format!("summon {kind} {} {} {}", x + dx, ground + 1.0, z + dz)).is_ok() { summoned += 1 }
        }
        self.cmd(format!("tp @s {} {} {} 0 60", x, ground + 14.0, z - 12.0))?;
        let mut m = self.measure(secs, |_| (self.pause)(1.0))?;
        m["mobs"] = json!(summoned);
        let _ = self.cmd("kill @e[type=!player,distance=..60]".into());
        Ok(m)
    }

    fn chests(&self, size: i64, height: i64, secs: f64) -> Result<Value, String> {
        let (x, _, z) = self.pos()?;
        let ground = self.ground(x, z)? as i64;
        let (x, z) = (x as i64 + 4, z as i64 + 4);
        let mut placed = 0;
        for layer in 0..height {
            let y = ground + layer;
            let r = self.cmd(format!("fill {x} {y} {z} {} {y} {} chest", x + size - 1, z + size - 1))?;
            if r["output"].as_array().is_some_and(|o| o.iter().any(|l| l.as_str().is_some_and(|l| l.contains("filled")))) { placed += size * size }
        }
        self.cmd(format!("tp @s {} {} {} -45 45", x - 4, ground + height + 6, z - 4))?;
        let mut m = self.measure(secs, |_| (self.pause)(1.0))?;
        m["chests"] = json!(placed);
        Ok(m)
    }
}
