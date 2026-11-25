//! Lock-free SPMC ring buffer with `Arc` frames.
//!
//! One writer appends frames; any number of readers hold independent cursors
//! and clone `Arc`s without copying frame data. Old frames are overwritten.
//!
//! Each slot uses `ArcSwapOption` for safe concurrent Arc replacement and an
//! `AtomicU64` sequence for publication/order.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::ArcSwapOption;

struct Slot<T> {
    seq: AtomicU64,
    data: ArcSwapOption<T>,
}

pub struct RingBuffer<T> {
    slots: Box<[Slot<T>]>,
    mask: usize,
    write: AtomicU64,
    dropped: AtomicU64,
}

pub struct RingReader<T> {
    ring: Arc<RingBuffer<T>>,
    cursor: u64,
}

impl<T: Send + Sync + 'static> RingBuffer<T> {
    pub fn new(capacity: usize) -> Arc<Self> {
        let capacity = capacity.max(2).next_power_of_two();
        let slots = (0..capacity)
            .map(|_| Slot {
                seq: AtomicU64::new(0),
                data: ArcSwapOption::empty(),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Arc::new(Self {
            slots,
            mask: capacity - 1,
            write: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        })
    }

    pub fn capacity(&self) -> usize {
        self.mask + 1
    }

    pub fn write_pos(&self) -> u64 {
        self.write.load(Ordering::Acquire)
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Append one frame. Never blocks readers; overwrites the oldest slot.
    pub fn push(&self, frame: Arc<T>) {
        let pos = self.write.fetch_add(1, Ordering::AcqRel) + 1;
        let slot = &self.slots[(pos as usize) & self.mask];
        slot.data.store(Some(frame));
        slot.seq.store(pos, Ordering::Release);
        if pos > self.capacity() as u64 {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Register a reader starting at the oldest still-reachable frame.
    pub fn subscribe(self: &Arc<Self>) -> RingReader<T> {
        let write = self.write.load(Ordering::Acquire);
        let start = write.saturating_sub(self.capacity() as u64);
        RingReader {
            ring: self.clone(),
            cursor: start,
        }
    }

    /// Register a reader starting at the current write position (live only).
    pub fn subscribe_latest(self: &Arc<Self>) -> RingReader<T> {
        RingReader {
            ring: self.clone(),
            cursor: self.write.load(Ordering::Acquire),
        }
    }

    fn try_read(&self, cursor: &mut u64) -> Option<Arc<T>> {
        let write = self.write.load(Ordering::Acquire);
        if *cursor >= write {
            return None;
        }
        let cap = self.capacity() as u64;
        if write - *cursor > cap {
            *cursor = write - cap;
        }
        let pos = *cursor + 1;
        let slot = &self.slots[(pos as usize) & self.mask];
        if slot.seq.load(Ordering::Acquire) != pos {
            return None;
        }
        let data = slot.data.load_full()?;
        *cursor = pos;
        Some(data)
    }
}

impl<T: Send + Sync + 'static> RingReader<T> {
    /// Non-blocking read.
    pub fn try_next(&mut self) -> Option<Arc<T>> {
        self.ring.try_read(&mut self.cursor)
    }

    /// Block briefly until a frame is available or the writer stalls.
    pub fn read(&mut self) -> Option<Arc<T>> {
        for _ in 0..10_000 {
            if let Some(frame) = self.try_next() {
                return Some(frame);
            }
            std::hint::spin_loop();
        }
        None
    }

    pub fn cursor(&self) -> u64 {
        self.cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read() {
        let ring = RingBuffer::new(4);
        let mut reader = ring.subscribe();
        ring.push(Arc::new(1u64));
        ring.push(Arc::new(2u64));
        assert_eq!(*reader.try_next().unwrap(), 1);
        assert_eq!(*reader.try_next().unwrap(), 2);
        assert!(reader.try_next().is_none());
    }

    #[test]
    fn overwrite_oldest() {
        let ring = RingBuffer::new(4);
        let mut reader = ring.subscribe();
        for i in 0..10u64 {
            ring.push(Arc::new(i));
        }
        assert_eq!(*reader.try_next().unwrap(), 6);
        assert_eq!(*reader.try_next().unwrap(), 7);
        assert_eq!(*reader.try_next().unwrap(), 8);
        assert_eq!(*reader.try_next().unwrap(), 9);
        assert!(reader.try_next().is_none());
        assert_eq!(ring.dropped(), 6);
    }

    #[test]
    fn multiple_readers() {
        let ring = RingBuffer::new(8);
        let mut a = ring.subscribe();
        let mut b = ring.subscribe();
        ring.push(Arc::new("x"));
        assert_eq!(*a.try_next().unwrap(), "x");
        assert_eq!(*b.try_next().unwrap(), "x");
    }

    #[test]
    fn concurrent_arc_sharing() {
        let ring = RingBuffer::new(1024);
        let mut reader = ring.subscribe();
        let handle = {
            let ring = ring.clone();
            std::thread::spawn(move || {
                for i in 0..100_000u64 {
                    ring.push(Arc::new(i));
                }
            })
        };
        handle.join().unwrap();
        let mut count = 0;
        while reader.try_next().is_some() {
            count += 1;
        }
        assert!(count > 0);
    }
}
