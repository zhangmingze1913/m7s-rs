//! Reusable byte buffer pool (foundation crate).

use std::sync::Mutex;

pub struct BytesPool {
    max: usize,
    free: Mutex<Vec<Vec<u8>>>,
}

impl BytesPool {
    pub fn new(max: usize) -> Self {
        Self { max: max.max(1), free: Mutex::new(Vec::new()) }
    }

    /// Get a buffer with at least `min` capacity.
    pub fn get(&self, min: usize) -> Vec<u8> {
        let mut free = self.free.lock().unwrap_or_else(|e| e.into_inner());
        while let Some(mut b) = free.pop() {
            if b.capacity() >= min {
                b.clear();
                return b;
            }
        }
        Vec::with_capacity(min)
    }

    pub fn put(&self, mut buf: Vec<u8>) {
        let mut free = self.free.lock().unwrap_or_else(|e| e.into_inner());
        if free.len() < self.max {
            buf.clear();
            free.push(buf);
        }
    }

    pub fn idle(&self) -> usize {
        self.free.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reuses_and_caps() {
        let p = BytesPool::new(1);
        let mut a = p.get(16);
        a.extend_from_slice(b"x");
        p.put(a);
        assert_eq!(p.idle(), 1);
        let b = p.get(8);
        assert!(b.capacity() >= 16);
        assert!(b.is_empty());
        p.put(b);
        p.put(Vec::with_capacity(4));
        assert_eq!(p.idle(), 1);
    }
}
