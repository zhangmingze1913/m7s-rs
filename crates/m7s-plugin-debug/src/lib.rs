//! Runtime debug snapshot.
use std::time::Instant;
static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
pub struct Snapshot { pub pid: u32, pub uptime_secs: u64, pub threads: usize }
pub fn snapshot() -> Snapshot {
    let start = START.get_or_init(Instant::now);
    Snapshot { pid: std::process::id(), uptime_secs: start.elapsed().as_secs(), threads: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) }
}
#[cfg(test)] mod tests { use super::*; #[test] fn pid_nonzero() { assert!(snapshot().pid > 0); } }
