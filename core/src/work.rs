//! What a slow job (copying a pack, taking or restoring a backup, importing) is doing right now, for the
//! window's progress bar. The app runs a job with `run`; code deep inside it calls `progress` as it goes.

use serde::Serialize;
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// One update: which job, what it's doing, how far (`total` 0 = can't tell), and whether it ended.
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Work {
    pub id: u64,
    pub label: String,
    pub done: usize,
    pub total: usize,
    pub end: bool,
}

type Sink = Box<dyn Fn(Work) + Send + Sync>;
static SINK: OnceLock<Sink> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);

thread_local! {
    // The job running on this thread, and when it last told the screen (updates are kept to a few a second).
    static JOB: RefCell<Option<(u64, String, Instant)>> = const { RefCell::new(None) };
}

/// Where updates go (the app sends them to the window). Set once.
pub fn set_sink(f: impl Fn(Work) + Send + Sync + 'static) {
    let _ = SINK.set(Box::new(f));
}

fn send(w: Work) {
    if let Some(s) = SINK.get() { s(w) }
}

/// Run `f` as a job the screen shows as `label`. Jobs inside a job just carry on under the outer one.
pub fn run<T>(label: &str, f: impl FnOnce() -> T) -> T {
    if JOB.with(|j| j.borrow().is_some()) {
        return f();
    }
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    JOB.with(|j| *j.borrow_mut() = Some((id, label.to_string(), Instant::now())));
    send(Work { id, label: label.into(), done: 0, total: 0, end: false });
    // Ended even if `f` panics, so the bar never hangs about.
    struct End(u64);
    impl Drop for End {
        fn drop(&mut self) {
            JOB.with(|j| *j.borrow_mut() = None);
            send(Work { id: self.0, label: String::new(), done: 0, total: 0, end: true });
        }
    }
    let _end = End(id);
    f()
}

/// How far the job on this thread is: `done` of `total`, with what it's on now (`what`, may be empty).
pub fn progress(what: &str, done: usize, total: usize) {
    JOB.with(|j| {
        let mut j = j.borrow_mut();
        let Some((id, label, last)) = j.as_mut() else { return };
        if done < total && last.elapsed() < Duration::from_millis(150) { return }
        *last = Instant::now();
        let label = if what.is_empty() { label.clone() } else { format!("{label} · {what}") };
        send(Work { id: *id, label, done, total, end: false });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_outside_a_job_is_ignored() {
        progress("x", 1, 2);
        assert_eq!(run("outer", || run("inner", || 7)), 7);
    }
}
