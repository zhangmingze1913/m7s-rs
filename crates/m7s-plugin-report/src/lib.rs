//! Counter-based report plugin.
use std::sync::atomic::{AtomicU64, Ordering};
#[derive(Default)]
pub struct Report { pub streams: AtomicU64, pub subscribers: AtomicU64, pub bytes: AtomicU64, pub errors: AtomicU64 }
impl Report {
    pub fn snapshot(&self) -> (u64,u64,u64,u64) { (self.streams.load(Ordering::Relaxed), self.subscribers.load(Ordering::Relaxed), self.bytes.load(Ordering::Relaxed), self.errors.load(Ordering::Relaxed)) }
    pub fn add_bytes(&self, n: u64) { self.bytes.fetch_add(n, Ordering::Relaxed); }
}
#[cfg(test)] mod tests { use super::*; #[test] fn add() { let r=Report::default(); r.add_bytes(5); assert_eq!(r.snapshot().2,5); } }
