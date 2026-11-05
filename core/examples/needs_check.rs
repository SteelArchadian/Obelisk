//! For a mods folder and a list of jars to turn off: what bisect would really turn off (mods others need put back),
//! and which jars left on would still miss a required mod id.
use obelisk_core::dupes::{needs, provides};
use std::collections::{BTreeSet, HashMap};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mods = std::path::Path::new(&a[1]);
    let mut off: BTreeSet<String> = std::fs::read_to_string(&a[2]).unwrap().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    let all: HashMap<String, (Vec<String>, Vec<String>)> = std::fs::read_dir(mods).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".jar")).map(|n| { let p = mods.join(&n); (n, (provides(&p), needs(&p))) }).collect();
    let asked = off.len();
    loop {
        let needed: BTreeSet<&String> = all.iter().filter(|(f, _)| !off.contains(*f)).flat_map(|(_, (_, n))| n).collect();
        let back: Vec<String> = off.iter().filter(|f| all.get(*f).is_some_and(|(ids, _)| ids.iter().any(|i| needed.contains(i)))).cloned().collect();
        if back.is_empty() { break }
        for f in back { off.remove(&f); }
    }
    println!("asked {asked}, really off {}", off.len());
    let have: BTreeSet<&String> = all.iter().filter(|(f, _)| !off.contains(*f)).flat_map(|(_, (p, _))| p).collect();
    let any: BTreeSet<&String> = all.values().flat_map(|(p, _)| p).collect();
    for (j, (_, n)) in all.iter().filter(|(f, _)| !off.contains(*f)) {
        let miss: Vec<_> = n.iter().filter(|n| !have.contains(n) && any.contains(n)).collect();
        if !miss.is_empty() { println!("{j}: missing {miss:?}") }
    }
}
