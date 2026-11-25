//! Global protocol connection limit.

use std::sync::atomic::{AtomicUsize, Ordering};

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

fn limit_from(value: Option<String>) -> usize {
    value.and_then(|v| v.parse().ok()).unwrap_or(0)
}

pub fn limit() -> usize {
    limit_from(std::env::var("M7S_MAX_CONNECTIONS").ok())
}

pub struct ConnectionGuard;

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn try_acquire() -> Option<ConnectionGuard> {
    let max = limit();
    if max == 0 {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        return Some(ConnectionGuard);
    }
    let mut current = ACTIVE.load(Ordering::SeqCst);
    loop {
        if current >= max {
            return None;
        }
        match ACTIVE.compare_exchange(current, current + 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return Some(ConnectionGuard),
            Err(actual) => current = actual,
        }
    }
}

pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_limit() {
        let base = ACTIVE.load(Ordering::SeqCst);
        let _ = base;
        unsafe { std::env::set_var("M7S_MAX_CONNECTIONS", "1"); }
        let a = try_acquire().unwrap();
        assert!(try_acquire().is_none());
        drop(a);
        assert!(try_acquire().is_some());
        unsafe { std::env::remove_var("M7S_MAX_CONNECTIONS"); }
    }
}
