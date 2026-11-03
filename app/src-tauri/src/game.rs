//! Talking to a running test game through the harness probe (testharness, -Dtestharness.probe=true): short JSON
//! answers instead of logs and screenshots. The game writes its port and token to <copy>/.obelisk/probe.json.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};

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
    s.set_read_timeout(Some(Duration::from_secs(20))).ok();
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

pub(crate) fn command(cmd: &str, args: &Value, dir: &Path, started: u64) -> Result<Value, String> {
    match cmd {
        "game-query" => query(dir, started),
        "game-cmd" => {
            let c = args.get("command").and_then(|c| c.as_str()).filter(|c| !c.trim().is_empty()).ok_or("missing \"command\"")?;
            ask(dir, started, json!({ "op": "cmd", "command": c }))
        }
        "game-scenario" => scenario(args, dir, started),
        _ => Err(format!("unknown command {cmd}")),
    }
}

fn nums(v: &Value) -> Option<Vec<f64>> {
    v.as_array()?.iter().map(|n| n.as_f64()).collect()
}

/// Run the steps in order and return one compact result; stops at the first failed step or assert.
fn scenario(args: &Value, dir: &Path, started: u64) -> Result<Value, String> {
    let steps = args.get("steps").and_then(|s| s.as_array()).filter(|s| !s.is_empty()).ok_or("missing \"steps\" (a list)")?;
    let limit = args.get("timeout").and_then(|t| t.as_u64()).unwrap_or(300).clamp(10, 600);
    let end = Instant::now() + Duration::from_secs(limit);
    let pause = |secs: f64| -> Result<(), String> {
        let d = Duration::from_secs_f64(secs.max(0.0));
        if Instant::now() + d > end { return Err(format!("the scenario would pass its {limit} s timeout")) }
        std::thread::sleep(d);
        Ok(())
    };
    let cmd = |c: String| ask(dir, started, json!({ "op": "cmd", "command": c }));
    let mut results = Vec::new();
    let mut last = Value::Null;
    for (i, step) in steps.iter().enumerate() {
        let r: Result<Value, String> = (|| {
            if let Some(c) = step.get("cmd").and_then(|c| c.as_str()) {
                return cmd(c.to_string());
            }
            if let Some(p) = step.get("tp").and_then(nums).filter(|p| p.len() == 3 || p.len() == 5) {
                let look = if p.len() == 5 { format!(" {} {}", p[3], p[4]) } else { String::new() };
                let r = cmd(format!("tp @s {} {} {}{look}", p[0], p[1], p[2]))?;
                pause(0.25)?; // the client sees the new position a tick later
                return Ok(r);
            }
            if let Some(w) = step.get("wait").and_then(|w| w.as_f64()) {
                pause(w)?;
                return Ok(json!({ "waited": w }));
            }
            if let Some(max) = step.get("wait_chunks").and_then(|w| w.as_f64()) {
                let t0 = Instant::now();
                loop {
                    let q = query(dir, started)?;
                    if q["renderReady"] == true { return Ok(json!({ "ready": true, "seconds": t0.elapsed().as_secs_f64().round() })) }
                    if t0.elapsed().as_secs_f64() >= max { return Ok(json!({ "ready": false, "seconds": max, "chunksLoaded": q["chunksLoaded"] })) }
                    pause(1.0)?;
                }
            }
            if let Some(f) = step.get("fly") {
                let path: Vec<Vec<f64>> = f.get("path").and_then(|p| p.as_array()).map(|p| p.iter().filter_map(nums).filter(|v| v.len() == 3).collect())
                    .filter(|p: &Vec<Vec<f64>>| p.len() >= 2).ok_or("fly needs path: two or more [x,y,z] points")?;
                let secs = f.get("seconds").and_then(|s| s.as_f64()).unwrap_or(20.0).max(1.0);
                let ticks = (secs * 4.0) as usize;
                for t in 0..=ticks {
                    let at = t as f64 / ticks as f64 * (path.len() - 1) as f64;
                    let (k, frac) = ((at.floor() as usize).min(path.len() - 2), at - at.floor().min((path.len() - 2) as f64));
                    let p: Vec<f64> = (0..3).map(|d| path[k][d] + (path[k + 1][d] - path[k][d]) * frac).collect();
                    cmd(format!("tp @s {:.2} {:.2} {:.2}", p[0], p[1], p[2]))?;
                    pause(0.25)?;
                }
                return Ok(json!({ "flown": secs }));
            }
            if let Some(secs) = step.get("sample").and_then(|s| s.as_f64()) {
                ask(dir, started, json!({ "op": "frames" }))?;
                let mut mspt = Vec::new();
                let t0 = Instant::now();
                while t0.elapsed().as_secs_f64() < secs {
                    pause(1.0_f64.min(secs - t0.elapsed().as_secs_f64()))?;
                    if let Some(m) = query(dir, started)?["mspt"].as_f64() { mspt.push(m) }
                }
                let mut f = ask(dir, started, json!({ "op": "frames" }))?;
                if !mspt.is_empty() {
                    f["mspt"] = json!((mspt.iter().sum::<f64>() / mspt.len() as f64 * 10.0).round() / 10.0);
                    f["msptMax"] = json!(mspt.iter().cloned().fold(0.0, f64::max));
                }
                return Ok(f);
            }
            if step.get("query").is_some() {
                return query(dir, started);
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
            Err(format!("unknown step {step}: use cmd, tp, wait, wait_chunks, fly, sample, query or assert"))
        })();
        match r {
            Ok(v) => {
                if step.get("assert").is_none() { last = v.clone() }
                results.push(json!({ "step": i, "result": v }));
            }
            Err(e) => return Ok(json!({ "ok": false, "failedStep": i, "error": e, "results": results })),
        }
    }
    Ok(json!({ "ok": true, "results": results }))
}
