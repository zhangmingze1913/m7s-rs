//! Minimal interval scheduler.
use std::time::{Duration, Instant};
pub struct Every { period: Duration, last: Instant }
impl Every {
    pub fn new(secs: u64) -> Self { Self { period: Duration::from_secs(secs.max(1)), last: Instant::now() } }
    pub fn due(&mut self) -> bool {
        if self.last.elapsed() >= self.period { self.last = Instant::now(); true } else { false }
    }
}
#[cfg(test)] mod tests { use super::*; #[test] fn not_due() { assert!(!Every::new(60).due()); } }
