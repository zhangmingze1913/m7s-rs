//! Per-stream Dispatcher: one push, N bounded subscriber channels, drop on full.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::{broadcast, mpsc};

use crate::flv::FlvTag;

pub type SharedTag = Arc<FlvTag>;

static DROPPED_TOTAL: AtomicU64 = AtomicU64::new(0);

pub fn dropped_total() -> u64 {
    DROPPED_TOTAL.load(Ordering::Relaxed)
}

/// Fan-out hub for one stream.
///
/// All viewers share one broadcast ring: publish cost is O(1), not
/// O(number of viewers). This is the main 10K/20K scaling property.
pub struct Dispatcher {
    /// Shared ring sender. Each receiver keeps its own cursor.
    tx: broadcast::Sender<SharedTag>,
    /// Ring capacity in tags.
    capacity: usize,
}

impl Dispatcher {
    pub fn new(capacity: usize) -> Arc<Self> {
        let (tx, _) = broadcast::channel(capacity.max(1));
        Arc::new(Self { tx, capacity })
    }

    /// Publish to all current subscribers. `send` returns Err when there are
    /// no receivers; that is normal before the first viewer.
    pub fn push(&self, tag: SharedTag) {
        let _ = self.tx.send(tag);
    }

    /// Register a viewer. `backlog` (cached GOP) is replayed before live tags
    /// so late joiners receive codec config and a keyframe first.
    pub fn subscribe(self: &Arc<Self>, backlog: Vec<SharedTag>) -> DispatchReader {
        let _ = self.capacity;
        DispatchReader {
            rx: self.tx.subscribe(),
            backlog: backlog.into(),
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn subscribers(&self) -> usize {
        self.tx.receiver_count()
    }
}

/// Per-viewer cursor into the shared broadcast ring.
pub struct DispatchReader {
    rx: broadcast::Receiver<SharedTag>,
    backlog: VecDeque<SharedTag>,
    dropped: Arc<AtomicU64>,
}

impl DispatchReader {
    pub fn try_next(&mut self) -> Option<SharedTag> {
        if let Some(tag) = self.backlog.pop_front() {
            return Some(tag);
        }
        loop {
            match self.rx.try_recv() {
                Ok(tag) => return Some(tag),
                Err(broadcast::error::TryRecvError::Empty) => return None,
                Err(broadcast::error::TryRecvError::Closed) => return None,
                Err(broadcast::error::TryRecvError::Lagged(n)) => {
                    self.dropped.fetch_add(n, Ordering::Relaxed);
                    DROPPED_TOTAL.fetch_add(n, Ordering::Relaxed);
                }
            }
        }
    }

    /// Wait for the next tag without polling.
    ///
    /// `Lagged` means this viewer fell behind the ring. We account the dropped
    /// count and continue from the oldest available tag; a slow client never
    /// blocks the publisher.
    pub async fn recv(&mut self) -> Option<SharedTag> {
        if let Some(tag) = self.backlog.pop_front() {
            return Some(tag);
        }
        loop {
            match self.rx.recv().await {
                Ok(tag) => return Some(tag),
                Err(broadcast::error::RecvError::Closed) => return None,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    self.dropped.fetch_add(n, Ordering::Relaxed);
                    DROPPED_TOTAL.fetch_add(n, Ordering::Relaxed);
                }
            }
        }
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}


/// Sharded dispatcher pool: publisher frames are routed to a worker by stream key.
pub struct DispatcherPool {
    workers: Vec<mpsc::Sender<(Arc<Dispatcher>, SharedTag)>>,
}

impl DispatcherPool {
    pub fn new(workers: usize) -> Arc<Self> {
        let n = workers.max(1);
        let mut tx = Vec::with_capacity(n);
        for _ in 0..n {
            let (sender, mut rx) = mpsc::channel::<(Arc<Dispatcher>, SharedTag)>(4096);
            tx.push(sender);
            tokio::spawn(async move {
                while let Some((d, tag)) = rx.recv().await {
                    d.push(tag);
                }
            });
        }
        Arc::new(Self { workers: tx })
    }

    pub fn worker_for(&self, key: &str) -> usize {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut h);
        (h.finish() as usize) % self.workers.len()
    }

    pub fn push(&self, key: &str, d: Arc<Dispatcher>, tag: SharedTag) {
        let idx = self.worker_for(key);
        self.push_at(idx, d, tag);
    }

    /// Publish with a precomputed shard index (hot path avoids re-hashing the key).
    pub fn push_at(&self, idx: usize, d: Arc<Dispatcher>, tag: SharedTag) {
        let idx = idx % self.workers.len();
        if let Err(mpsc::error::TrySendError::Full((d, tag))) =
            self.workers[idx].try_send((d, tag))
        {
            // Slow worker: fall back to inline dispatch instead of dropping.
            d.push(tag);
        }
    }
}

static POOL: std::sync::OnceLock<Option<Arc<DispatcherPool>>> = std::sync::OnceLock::new();

pub fn init_pool(workers: usize) {
    let _ = POOL.set(if workers == 0 {
        None
    } else {
        Some(DispatcherPool::new(workers))
    });
}

pub fn pool() -> Option<&'static Arc<DispatcherPool>> {
    POOL.get().and_then(|p| p.as_ref())
}

#[cfg(test)]
mod pool_tests {
    use super::*;

    fn tag(ts: u32) -> SharedTag {
        Arc::new(FlvTag::new(9, ts, vec![1]))
    }
    #[test]
    fn routes_and_delivers() {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let d = Dispatcher::new(10);
                let mut r = d.subscribe(Vec::new());
                let pool = DispatcherPool::new(2);
                for i in 0..8 {
                    pool.push("live/test", d.clone(), tag(i));
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let mut got = Vec::new();
                while let Some(t) = r.try_next() {
                    got.push(t.timestamp);
                }
                assert_eq!(got, (0..8).collect::<Vec<_>>());
            });
    }

    #[test]
    fn same_key_same_worker() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let pool = DispatcherPool::new(4);
                assert_eq!(pool.worker_for("live/a"), pool.worker_for("live/a"));
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(ts: u32) -> SharedTag {
        Arc::new(FlvTag::new(9, ts, vec![1, 2, 3]))
    }

    #[test]
    fn broadcasts_to_all() {
        let d = Dispatcher::new(150);
        let mut a = d.subscribe(Vec::new());
        let mut b = d.subscribe(Vec::new());
        d.push(tag(1));
        assert_eq!(a.try_next().unwrap().timestamp, 1);
        assert_eq!(b.try_next().unwrap().timestamp, 1);
    }

    #[test]
    fn backlog_precedes_live() {
        let d = Dispatcher::new(150);
        let mut r = d.subscribe(vec![tag(7)]);
        d.push(tag(8));
        assert_eq!(r.try_next().unwrap().timestamp, 7);
        assert_eq!(r.try_next().unwrap().timestamp, 8);
    }

    #[test]
    fn drops_when_lagged_and_unsubscribes() {
        let d = Dispatcher::new(2);
        let mut slow = d.subscribe(Vec::new());
        for i in 0..5 {
            d.push(tag(i));
        }
        assert!(slow.try_next().is_some());
        assert!(slow.dropped() >= 1);
        drop(slow);
        assert_eq!(d.subscribers(), 0);
    }

    #[tokio::test]
    async fn async_recv_gets_live_tag() {
        let d = Dispatcher::new(4);
        let mut r = d.subscribe(Vec::new());
        d.push(tag(9));
        assert_eq!(r.recv().await.unwrap().timestamp, 9);
    }
}
