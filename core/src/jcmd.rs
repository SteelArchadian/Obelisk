//! jcmd against a running game (heap, thread dumps), never left hanging when the game is frozen or gone.

use std::path::Path;
use std::time::{Duration, Instant};

/// A process is still running (not ended, not a stale pid).
pub fn pid_alive(pid: u32) -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn GetExitCodeProcess(h: isize, code: *mut u32) -> i32;
        fn CloseHandle(h: isize) -> i32;
    }
    // PROCESS_QUERY_LIMITED_INFORMATION; STILL_ACTIVE is 259.
    let h = unsafe { OpenProcess(0x1000, 0, pid) };
    if h == 0 {
        return false;
    }
    let mut code = 0u32;
    let ok = unsafe { GetExitCodeProcess(h, &mut code) } != 0;
    unsafe { CloseHandle(h) };
    ok && code == 259
}

/// jcmd against a game, never left behind: skipped when the game is gone, killed when it doesn't answer in
/// `limit` (a frozen or dying game makes jcmd's attach wait forever; 2026-11-24 about 40 of them piled up).
/// (succeeded, stdout).
pub fn jcmd_run(jcmd: &Path, pid: u32, args: &[&str], limit: Duration) -> Result<(bool, String), String> {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    if !pid_alive(pid) {
        return Err(format!("jcmd: the game (pid {pid}) is not running"));
    }
    let mut child = std::process::Command::new(jcmd).arg(pid.to_string()).args(args)
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null())
        .creation_flags(0x0800_0000).spawn().map_err(|e| format!("jcmd: {e}"))?;
    // Read on the side, so a big thread dump can't fill the pipe and stall jcmd.
    let mut pipe = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = pipe.as_mut() { let _ = p.read_to_end(&mut buf); }
        buf
    });
    let t0 = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if t0.elapsed() < limit => std::thread::sleep(Duration::from_millis(100)),
            _ => break None,
        }
    };
    let Some(status) = status else {
        let _ = child.kill();
        let _ = child.wait();
        let _ = reader.join();
        return Err(format!("jcmd: the game didn't answer within {} s (frozen?); stopped", limit.as_secs()));
    };
    let out = String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned();
    Ok((status.success(), out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jdk21_jcmd() -> Option<std::path::PathBuf> {
        std::fs::read_dir("C:/Program Files/Eclipse Adoptium").ok()?.flatten().map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("jdk-21")))
            .map(|p| p.join("bin").join("jcmd.exe")).find(|p| p.exists())
    }

    fn jcmds() -> usize {
        let o = std::process::Command::new("tasklist").args(["/FI", "IMAGENAME eq jcmd.exe", "/FO", "CSV", "/NH"]).output().unwrap();
        String::from_utf8_lossy(&o.stdout).lines().filter(|l| l.to_ascii_lowercase().contains("jcmd.exe")).count()
    }

    /// Hostile: a game that's gone, and one frozen solid (suspended, so jcmd's attach never gets an answer). Neither
    /// may leave a jcmd behind.
    #[test]
    fn jcmd_never_hangs_on_a_dead_or_frozen_game() {
        use std::time::{Duration, Instant};
        let Some(jcmd) = jdk21_jcmd() else { return eprintln!("no JDK 21: skipped") };
        // Gone: refused straight away, nothing started.
        let dead = std::process::Command::new("cmd").args(["/c", "exit"]).spawn().unwrap();
        let pid = dead.id();
        let _ = dead.wait_with_output();
        let t0 = Instant::now();
        assert!(super::jcmd_run(&jcmd, pid, &["GC.heap_info"], Duration::from_secs(5)).is_err());
        assert!(t0.elapsed() < Duration::from_secs(1));
        // Frozen: a Java that sleeps, suspended whole.
        let dir = std::env::temp_dir().join("obelisk-jcmd-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("S.java"), "class S { public static void main(String[] a) throws Exception { Thread.sleep(120000); } }").unwrap();
        let mut java = std::process::Command::new(jcmd.with_file_name("java.exe")).arg(dir.join("S.java")).spawn().unwrap();
        let jpid = java.id();
        std::thread::sleep(Duration::from_secs(4));
        assert!(super::jcmd_run(&jcmd, jpid, &["VM.uptime"], Duration::from_secs(20)).unwrap().0, "live attach works");
        #[link(name = "ntdll")]
        unsafe extern "system" { fn NtSuspendProcess(h: isize) -> i32; }
        #[link(name = "kernel32")]
        unsafe extern "system" { fn OpenProcess(a: u32, i: i32, p: u32) -> isize; fn CloseHandle(h: isize) -> i32; }
        let h = unsafe { OpenProcess(0x0800, 0, jpid) }; // PROCESS_SUSPEND_RESUME
        assert!(h != 0);
        unsafe { NtSuspendProcess(h); CloseHandle(h); }
        let before = jcmds();
        let t0 = Instant::now();
        let r = super::jcmd_run(&jcmd, jpid, &["GC.heap_info"], Duration::from_secs(3));
        assert!(r.is_err(), "frozen game must time out: {r:?}");
        assert!(t0.elapsed() < Duration::from_secs(6));
        // The timed-out jcmd was stopped, not left waiting.
        let left = jcmds();
        let _ = java.kill();
        let _ = java.wait();
        assert!(left <= before, "jcmd left behind: {before} before, {left} after");
    }

}
