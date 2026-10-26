//! AI dev mode: what a test run looks like from outside the game, from the moment Java starts.
//! Once a second: the game's CPU, memory, GPU load and video memory; every 10 s the Java heap (jcmd).
//! Stage events (mods loaded, resources, title screen, world join, terrain ready, quit) come from the log and
//! Optimal's timeline as they happen. Everything goes to <copy>/obelisk-runs/<job>.samples.jsonl.

use obelisk_core as prism;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Stage names a chat can wait for, in the order they normally happen.
/// Log marks and Optimal's events share these names (Optimal's are exact; "titleScreen", "terrainReady" and
/// "pressure0".."pressure3" come only from Optimal, "window" from Obelisk seeing the game's window).
pub const STAGES: &[&str] = &["logStart", "window", "modsLoaded", "resourcesLoaded", "titleScreen", "joinStart", "inWorld", "terrainReady",
    "leaveStart", "leaveDone", "quit"];

pub struct Sampler {
    pid: u32,
    dir: PathBuf,
    launched_ms: i64,
    out: Option<std::fs::File>,
    sys: sysinfo::System,
    gpu: Option<gpu::Query>,
    heap: Arc<Mutex<Option<(f64, f64)>>>,
    alive: Arc<std::sync::atomic::AtomicBool>,
    last_sample: Instant,
    last_events: Instant,
    seen: std::collections::HashSet<(String, i64)>,
    pub events: Vec<Value>,
    pub latest: Value,
}

impl Sampler {
    pub fn new(pid: u32, dir: &Path, job: u32, launched_ms: i64, jcmd: Option<PathBuf>) -> Self {
        let out = std::fs::File::create(dir.join("obelisk-runs").join(format!("{job}.samples.jsonl"))).ok();
        let heap = Arc::new(Mutex::new(None));
        let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        if let Some(jcmd) = jcmd {
            let (heap, alive) = (heap.clone(), alive.clone());
            // jcmd takes up to a second; its own thread keeps the 1 s samples on time.
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(5));
                while alive.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Some(h) = heap_info(&jcmd, pid) {
                        *heap.lock().unwrap() = Some(h);
                    }
                    std::thread::sleep(Duration::from_secs(10));
                }
            });
        }
        let past = Instant::now() - Duration::from_secs(60);
        Sampler { pid, dir: dir.to_path_buf(), launched_ms, out, sys: sysinfo::System::new(), gpu: gpu::Query::new(),
            heap, alive, last_sample: past, last_events: past, seen: Default::default(), events: vec![], latest: Value::Null }
    }

    /// Call often; samples once a second and looks for new events every 2 s. True when something new was found.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        if self.last_sample.elapsed() >= Duration::from_secs(1) {
            self.last_sample = Instant::now();
            let pid = sysinfo::Pid::from_u32(self.pid);
            self.sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[pid]), true,
                sysinfo::ProcessRefreshKind::new().with_cpu().with_memory());
            let (cpu, mem) = self.sys.process(pid).map(|p| (p.cpu_usage() as f64, p.memory() as f64 / 1_048_576.0)).unwrap_or((0.0, 0.0));
            let (gpu, vram) = self.gpu.as_mut().map(|q| q.read(self.pid)).unwrap_or((None, None));
            let heap = *self.heap.lock().unwrap();
            let r1 = |x: f64| (x * 10.0).round() / 10.0;
            self.latest = json!({
                "t": r1((prism::timeline::now_ms() - self.launched_ms) as f64 / 1000.0),
                // CPU in cores (100 = one core busy), memory = working set.
                "cpu": r1(cpu), "memMB": mem.round(), "gpu3d": gpu.map(r1), "vramMB": vram.map(f64::round),
                "heapUsedMB": heap.map(|h| h.0.round()), "heapCommittedMB": heap.map(|h| h.1.round()),
            });
            self.write(&self.latest.clone());
        }
        if self.last_events.elapsed() >= Duration::from_secs(2) {
            self.last_events = Instant::now();
            for m in prism::timeline::live_marks(&self.dir, self.launched_ms) {
                if !self.seen.insert((m.name.clone(), m.ms)) { continue; }
                let (exact, name) = match m.name.strip_prefix("sl.") { Some(n) => (true, n), None => (false, m.name.as_str()) };
                let name = match name {
                    "gameStart" => "modsLoaded", "worldStart" => "joinStart", "worldLeave" => "leaveStart",
                    "worldSaved" => "leaveDone", "quitStart" => "quit", n => n,
                };
                self.add(name, m.ms, if exact { "optimal" } else { "log" });
                changed = true;
            }
        }
        changed
    }

    pub fn add(&mut self, name: &str, ms: i64, src: &str) {
        let e = json!({ "event": name, "t": ((ms - self.launched_ms) as f64 / 100.0).round() / 10.0, "src": src });
        self.write(&e);
        self.events.push(e);
    }

    fn write(&mut self, v: &Value) {
        if let Some(f) = self.out.as_mut() {
            let _ = writeln!(f, "{v}");
        }
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.alive.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

/// jcmd for a game's java.exe: the one next to it, else an installed JDK of the same version
/// (Mojang's Java 21 runtime has no jcmd; an installed JDK 21 attaches fine).
pub fn jcmd_for(java: &str) -> Result<PathBuf, String> {
    let major = if java.contains("jdk-25") { "jdk-25" } else { "jdk-21" };
    let installed = std::fs::read_dir("C:/Program Files/Eclipse Adoptium").into_iter().flatten().flatten()
        .map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(major)))
        .map(|p| p.join("bin").join("jcmd.exe")).filter(|p| p.exists()).max();
    Some(Path::new(java).with_file_name("jcmd.exe")).filter(|p| p.exists()).or(installed)
        .ok_or_else(|| format!("no jcmd: none next to {java} and no {major}* JDK under C:/Program Files/Eclipse Adoptium"))
}

/// (used, committed) heap in MB from `jcmd GC.heap_info` (ZGC: "used 1234M, capacity 2048M"; G1: "total 524288K, used 123456K").
fn heap_info(jcmd: &Path, pid: u32) -> Option<(f64, f64)> {
    use super::NoWindow;
    let o = std::process::Command::new(jcmd).args([&pid.to_string(), "GC.heap_info"]).creation_flags_no_window().output().ok()?;
    parse_heap_info(&String::from_utf8_lossy(&o.stdout))
}

fn parse_heap_info(text: &str) -> Option<(f64, f64)> {
    let size = |s: &str, key: &str| -> Option<f64> {
        let rest = &s[s.find(key)? + key.len()..];
        let n: String = rest.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
        let unit = rest.trim_start()[n.len()..].chars().next()?;
        let v: f64 = n.parse().ok()?;
        Some(match unit { 'K' => v / 1024.0, 'M' => v, 'G' => v * 1024.0, _ => v / 1_048_576.0 })
    };
    let line = text.lines().find(|l| l.contains("used "))?;
    Some((size(line, "used ")?, size(line, "capacity ").or_else(|| size(line, "total "))?))
}

/// Per-process GPU load and video memory through the Windows performance counters (Task Manager's numbers).
mod gpu {
    #[link(name = "pdh")]
    unsafe extern "system" {
        fn PdhOpenQueryW(src: *const u16, user: usize, q: *mut isize) -> u32;
        fn PdhAddEnglishCounterW(q: isize, path: *const u16, user: usize, c: *mut isize) -> u32;
        fn PdhCollectQueryData(q: isize) -> u32;
        fn PdhGetFormattedCounterArrayW(c: isize, fmt: u32, size: *mut u32, count: *mut u32, items: *mut u8) -> u32;
        fn PdhCloseQuery(q: isize) -> u32;
    }
    const PDH_FMT_DOUBLE: u32 = 0x200;
    const PDH_MORE_DATA: u32 = 0x800007D2;

    pub struct Query { q: isize, engine: isize, memory: isize }
    unsafe impl Send for Query {}

    impl Query {
        pub fn new() -> Option<Self> {
            let w = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
            let (mut q, mut engine, mut memory) = (0, 0, 0);
            unsafe {
                if PdhOpenQueryW(std::ptr::null(), 0, &mut q) != 0 { return None; }
                if PdhAddEnglishCounterW(q, w(r"\GPU Engine(*)\Utilization Percentage").as_ptr(), 0, &mut engine) != 0
                    || PdhAddEnglishCounterW(q, w(r"\GPU Process Memory(*)\Dedicated Usage").as_ptr(), 0, &mut memory) != 0 {
                    PdhCloseQuery(q);
                    return None;
                }
                PdhCollectQueryData(q);
            }
            Some(Query { q, engine, memory })
        }

        /// (3D engine load %, dedicated video memory MB) for one process.
        pub fn read(&mut self, pid: u32) -> (Option<f64>, Option<f64>) {
            unsafe { PdhCollectQueryData(self.q) };
            let tag = format!("pid_{pid}_");
            let load = items(self.engine).filter(|(n, _)| n.starts_with(&tag) && n.ends_with("engtype_3D")).map(|(_, v)| v).reduce(|a, b| a + b);
            let mem = items(self.memory).filter(|(n, _)| n.starts_with(&tag)).map(|(_, v)| v / 1_048_576.0).reduce(|a, b| a + b);
            (load.map(|l| l.min(100.0)), mem)
        }
    }

    impl Drop for Query {
        fn drop(&mut self) {
            unsafe { PdhCloseQuery(self.q) };
        }
    }

    /// PDH_FMT_COUNTERVALUE_ITEM_W: name pointer, then {status u32, pad, double}.
    fn items(c: isize) -> impl Iterator<Item = (String, f64)> {
        let (mut size, mut count) = (0u32, 0u32);
        let mut out = vec![];
        unsafe {
            if PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, std::ptr::null_mut()) == PDH_MORE_DATA {
                let mut buf = vec![0u64; (size as usize).div_ceil(8)];
                if PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, buf.as_mut_ptr() as *mut u8) == 0 {
                    let base = buf.as_ptr() as *const u8;
                    for i in 0..count as usize {
                        let item = base.add(i * 24);
                        let name = *(item as *const *const u16);
                        let status = *(item.add(8) as *const u32);
                        let value = *(item.add(16) as *const f64);
                        if name.is_null() || status > 1 { continue; }
                        let len = (0..).take_while(|&k| *name.add(k) != 0).count();
                        out.push((String::from_utf16_lossy(std::slice::from_raw_parts(name, len)), value));
                    }
                }
            }
        }
        out.into_iter()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn heap_info_zgc_and_g1() {
        let z = " ZHeap           used 1234M, capacity 2048M, max capacity 8192M\n Metaspace       used 300M";
        assert_eq!(super::parse_heap_info(z), Some((1234.0, 2048.0)));
        let g = " garbage-first heap   total 524288K, used 131072K [0x0000000600000000, 0x0000000800000000)";
        assert_eq!(super::parse_heap_info(g), Some((128.0, 512.0)));
    }

    #[test]
    fn gpu_counters_open() {
        let mut q = super::gpu::Query::new().expect("PDH GPU counters");
        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = q.read(std::process::id());
    }
}
