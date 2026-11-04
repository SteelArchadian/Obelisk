//! Thanos: find the mod behind a behaviour by halving. Each round runs the chat's scenario (a game_test) with half of
//! the suspects disabled for that run only; the chat looks at the result (shots, numbers) and answers whether the
//! behaviour showed. Mods a kept mod requires stay on, so every run can load. One bisect per copy, kept on disk.

use super::control::{self as c, Shared};
use obelisk_core as prism;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// The harness and the bench's own mods are never suspects: the runs need them.
const BENCH: &[&str] = &["testharness", "speedloader", "optimal", "lightning", "blockdyno"];

type Jars = HashMap<String, (Vec<String>, Vec<String>)>;

fn file(copy: &str) -> PathBuf {
    super::datadir::root().join("bisect").join(format!("{copy}.json"))
}

fn load(copy: &str) -> Result<Value, String> {
    std::fs::read(file(copy)).ok().and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| format!("no bisect in {copy}; start one with action start"))
}

fn save(copy: &str, st: &Value) -> Result<(), String> {
    let f = file(copy);
    std::fs::create_dir_all(f.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&f, serde_json::to_vec_pretty(st).unwrap()).map_err(|e| e.to_string())
}

/// Enabled jars: file -> (ids it provides, ids it requires).
fn jars(dir: &Path) -> Jars {
    std::fs::read_dir(dir.join("mods")).into_iter().flatten().flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(n, _)| n.to_lowercase().ends_with(".jar"))
        .map(|(n, p)| (n, (prism::dupes::own_ids(&p), prism::dupes::needs(&p))))
        .collect()
}

/// From `off`, the jars that can really be off: any that provide an id a jar left on requires are put back on.
fn loadable(all: &Jars, mut off: BTreeSet<String>) -> BTreeSet<String> {
    loop {
        let needed: BTreeSet<&String> = all.iter().filter(|(f, _)| !off.contains(*f)).flat_map(|(_, (_, n))| n).collect();
        let back: Vec<String> = off.iter().filter(|f| all.get(*f).is_some_and(|(ids, _)| ids.iter().any(|i| needed.contains(i)))).cloned().collect();
        if back.is_empty() { return off }
        for f in back { off.remove(&f); }
    }
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|s| s.as_str().map(String::from)).collect()
}

/// Plan the next split of the suspects and start its run.
fn round(copy: &str, dir: &Path, mut st: Value, jobs: &Shared) -> Result<Value, String> {
    let suspects = strs(&st["suspects"]);
    let all = jars(dir);
    let half = suspects.len().div_ceil(2);
    let mut off = loadable(&all, suspects[half..].iter().cloned().collect());
    if off.is_empty() { off = loadable(&all, suspects[..half].iter().cloned().collect()) }
    if off.is_empty() {
        st["state"] = json!("stuck");
        save(copy, &st)?;
        return Ok(json!({ "state": "stuck", "suspects": suspects,
            "note": "every suspect is required by another mod, so none can be turned off alone; test these by hand (mod_set in a copy)" }));
    }
    let on: Vec<String> = suspects.iter().filter(|s| !off.contains(*s)).cloned().collect();
    let mut run = st["run"].clone();
    let mut disabled = strs(&run["disable_mods"]);
    disabled.extend(off.iter().cloned());
    run["disable_mods"] = json!(disabled);
    run["copy"] = json!(copy);
    run["background"] = json!(true);
    let r = c::game_test(&run, jobs)?;
    let n = st["round"].as_u64().unwrap_or(0) + 1;
    st["round"] = json!(n);
    st["job"] = r["job"].clone();
    st["on"] = json!(on);
    st["off"] = json!(off);
    st["state"] = json!("running");
    save(copy, &st)?;
    Ok(json!({ "state": "running", "round": n, "job": r["job"], "suspects": suspects.len(), "suspects_on": on.len(), "disabled": off.len(),
        "rounds_left_about": (suspects.len() as f64).log2().ceil(),
        "next": "game_result {job} until done; look at its result and shots; then bisect {copy, action: answer, present: true|false} (failed: true if the run broke)" }))
}

pub(crate) fn command(args: &Value, jobs: &Shared) -> Result<Value, String> {
    let copy = c::need(args, "copy")?.to_string();
    if copy == "play" { return Err("bisect runs in a test copy, never the play instance".into()) }
    let dir = c::test_copy(&copy)?;
    match c::need(args, "action")? {
        "start" => {
            let run = args.get("run").filter(|r| r.is_object()).cloned().ok_or("run: the game_test arguments for each round (owner, purpose, preset, steps...)")?;
            for k in ["owner", "purpose"] { if run[k].as_str().is_none() { return Err(format!("run needs {k}")) } }
            let keep: Vec<String> = strs(&args["keep"]).iter().map(|s| s.to_lowercase()).collect();
            let skip = strs(&run["disable_mods"]);
            let mut suspects: Vec<String> = jars(&dir).into_keys()
                .filter(|f| { let l = f.to_lowercase(); !BENCH.iter().any(|b| l.starts_with(b)) && !keep.iter().any(|k| l.starts_with(k)) && !skip.contains(f) })
                .collect();
            if let Some(only) = args.get("suspects").filter(|v| v.is_array()) {
                let only = strs(only);
                suspects.retain(|f| only.contains(f));
            }
            suspects.sort();
            if suspects.len() < 2 { return Err(format!("{} suspect(s): nothing to split", suspects.len())) }
            let st = json!({ "copy": copy, "run": run, "suspects": suspects, "round": 0, "history": [] });
            round(&copy, &dir, st, jobs)
        }
        "answer" => {
            let mut st = load(&copy)?;
            if st["state"] != "running" { return Err(format!("the bisect is {}, not waiting for an answer", st["state"])) }
            if args["failed"].as_bool() == Some(true) {
                st["state"] = json!("failed");
                save(&copy, &st)?;
                return Ok(json!({ "state": "failed", "note": "the run broke: read game_result / diagnose; retry runs the same split again, stop ends the bisect" }));
            }
            let present = args["present"].as_bool().ok_or("present: true if the behaviour showed in this run, false if not")?;
            let next = if present { st["on"].clone() } else { st["off"].clone() };
            let entry = json!({ "round": st["round"], "job": st["job"], "present": present, "on": strs(&st["on"]).len(), "off": strs(&st["off"]).len() });
            if let Some(h) = st["history"].as_array_mut() { h.push(entry) }
            st["suspects"] = next.clone();
            let left = strs(&next);
            if left.len() <= 1 {
                st["state"] = json!("done");
                save(&copy, &st)?;
                return Ok(json!({ "state": "done", "culprit": left.first(), "rounds": st["round"],
                    "note": "confirm it: run the scenario once with only this mod disabled (game_test disable_mods)" }));
            }
            round(&copy, &dir, st, jobs)
        }
        "retry" => {
            let mut st = load(&copy)?;
            st["round"] = json!(st["round"].as_u64().unwrap_or(1).saturating_sub(1));
            round(&copy, &dir, st, jobs)
        }
        "status" => {
            let st = load(&copy)?;
            let n = strs(&st["suspects"]).len();
            Ok(json!({ "state": st["state"], "round": st["round"], "job": st["job"], "suspects": n,
                "on": st["on"], "off": st["off"], "history": st["history"], "list": (n <= 16).then(|| st["suspects"].clone()) }))
        }
        "stop" => {
            let st = load(&copy)?;
            let _ = std::fs::remove_file(file(&copy));
            Ok(json!({ "stopped": true, "suspects": st["suspects"] }))
        }
        a => Err(format!("action {a}: start, answer, retry, status or stop")),
    }
}
