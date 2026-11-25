//! Stream registry with a GOP cache and broadcast fan-out.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::flv::FlvTag;
use crate::time::{now_ms, rfc3339};

pub type SharedTag = Arc<FlvTag>;

pub struct Stream {
    pub app: String,
    pub name: String,
    key: String,
    cache: RwLock<VecDeque<SharedTag>>,
    metadata: RwLock<Option<SharedTag>>,
    configs: RwLock<Vec<SharedTag>>,
    tx: broadcast::Sender<SharedTag>,
    subscribers: AtomicUsize,
    bytes: AtomicU64,
    latencies: Mutex<VecDeque<u32>>,
    started_ms: i64,
    gop_cache: usize,
    last_ts: Mutex<u32>,
    ring: Arc<crate::ring::RingBuffer<FlvTag>>,
    tap_tx: broadcast::Sender<m7s_sdk::SharedTag>,
    dispatcher: Arc<crate::dispatcher::Dispatcher>,
}

impl Stream {
    pub fn new(app: &str, name: &str, gop_cache: usize) -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self {
            app: app.to_string(),
            name: name.to_string(),
            key: format!("{app}/{name}"),
            cache: RwLock::new(VecDeque::with_capacity(gop_cache)),
            metadata: RwLock::new(None),
            configs: RwLock::new(Vec::new()),
            tx,
            subscribers: AtomicUsize::new(0),
            bytes: AtomicU64::new(0),
            latencies: Mutex::new(VecDeque::new()),
            started_ms: now_ms(),
            gop_cache: gop_cache.max(1),
            last_ts: Mutex::new(0),
            ring: crate::ring::RingBuffer::new(gop_cache.saturating_mul(2).max(64)),
            tap_tx: broadcast::channel(1024).0,
            dispatcher: crate::dispatcher::Dispatcher::new(150),
        }
    }

    pub fn key(&self) -> String {
        format!("{}/{}", self.app, self.name)
    }

    /// Publish one media/metadata tag into the stream.
    ///
    /// Order matters: cache/config must be updated before fan-out so a viewer
    /// that subscribes concurrently can still obtain codec config from backlog.
    pub fn push(&self, tag: SharedTag) {
        let mut tag = tag;
        {
            let mut last = self.last_ts.lock().unwrap_or_else(|e| e.into_inner());
            if tag.timestamp.saturating_add(1000) < *last {
                let mut fixed = (*tag).clone();
                fixed.timestamp = last.saturating_add(40);
                tag = Arc::new(fixed);
            }
            if tag.timestamp > *last {
                *last = tag.timestamp;
            }
        }
        // Append to the per-stream history ring used by debug/backlog paths.
        self.ring.push(tag.clone());
        if self.tap_tx.receiver_count() > 0 {
            let _ = self.tap_tx.send(Arc::new(m7s_sdk::Tag { tag_type: tag.tag_type, timestamp: tag.timestamp, data: tag.data.clone() }));
        }
        self.bytes
            .fetch_add(tag.data.len() as u64 + 11, Ordering::Relaxed);
        // AVC/AAC sequence headers replace any previous config of same type.
        if (tag.tag_type == 9 || tag.tag_type == 8) && tag.data.len() > 1 && tag.data[1] == 0 {
            let mut configs = self.configs.write().unwrap_or_else(|e| e.into_inner());
            configs.retain(|t| t.tag_type != tag.tag_type);
            configs.push(tag.clone());
        }
        if tag.tag_type == 18 {
            *self.metadata.write().unwrap_or_else(|e| e.into_inner()) = Some(tag.clone());
        }
        {
            let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
            cache.push_back(tag.clone());
            while cache.len() > self.gop_cache {
                cache.pop_front();
            }
        }
        // Fan-out is last: readers must be able to see the cache/config above.
        if let Some(pool) = crate::dispatcher::pool() {
            pool.push(&self.key, self.dispatcher.clone(), tag.clone());
        } else {
            self.dispatcher.push(tag.clone());
        }
        let _ = self.tx.send(tag);
    }

    /// Recent GOP snapshot for SDP/config probing without registering a subscriber.
    pub fn cache_snapshot(&self) -> Vec<SharedTag> {
        let mut out: Vec<SharedTag> = self
            .configs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect();
        out.extend(
            self.cache
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .cloned(),
        );
        out
    }

    pub fn ring_reader_latest(&self) -> crate::ring::RingReader<FlvTag> {
        self.subscribers.fetch_add(1, Ordering::Relaxed);
        self.ring.subscribe_latest()
    }

    pub fn dispatch_reader(&self) -> crate::dispatcher::DispatchReader {
        self.subscribers.fetch_add(1, Ordering::Relaxed);
        self.dispatcher.subscribe(self.cache_snapshot())
    }

    pub fn ring_reader(&self) -> crate::ring::RingReader<FlvTag> {
        self.subscribers.fetch_add(1, Ordering::Relaxed);
        self.ring.subscribe()
    }

    pub fn subscribe(&self) -> (Vec<SharedTag>, broadcast::Receiver<SharedTag>) {
        self.subscribers.fetch_add(1, Ordering::Relaxed);
        let mut cache: Vec<SharedTag> = self
            .cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect();
        for config in self
            .configs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .rev()
        {
            if !cache.iter().any(|t| Arc::ptr_eq(t, config)) {
                cache.insert(0, config.clone());
            }
        }
        (cache, self.tx.subscribe())
    }

    pub fn leave(&self) {
        self.subscribers.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn record_latency(&self, micros: u32) {
        let mut values = self.latencies.lock().unwrap_or_else(|e| e.into_inner());
        values.push_back(micros);
        while values.len() > 4096 {
            values.pop_front();
        }
    }

    fn latency_p99(&self) -> u64 {
        let values = self.latencies.lock().unwrap_or_else(|e| e.into_inner());
        if values.is_empty() {
            return 0;
        }
        let mut sorted: Vec<u32> = values.iter().copied().collect();
        sorted.sort_unstable();
        let idx = (sorted.len() * 99 / 100).min(sorted.len() - 1);
        sorted[idx] as u64
    }

    pub fn metadata(&self) -> Option<SharedTag> {
        self.metadata
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn json(&self) -> Value {
        json!({
            "app": self.app,
            "name": self.name,
            "key": self.key(),
            "subscribers": self.subscribers.load(Ordering::Relaxed),
            "bytes": self.bytes.load(Ordering::Relaxed),
            "started": rfc3339(self.started_ms),
            "cached_tags": self.cache.read().unwrap_or_else(|e| e.into_inner()).len(),
            "latency_p99_us": self.latency_p99(),
            "latency_samples": self.latencies.lock().unwrap_or_else(|e| e.into_inner()).len(),
        })
    }
}

pub struct StreamManager {
    streams: RwLock<HashMap<String, Arc<Stream>>>,
    gop_cache: usize,
}

impl StreamManager {
    pub fn new(gop_cache: usize) -> Arc<Self> {
        Arc::new(Self {
            streams: RwLock::new(HashMap::new()),
            gop_cache,
        })
    }

    pub fn try_get_or_create(&self, app: &str, name: &str) -> Option<Arc<Stream>> {
        let key = format!("{app}/{name}");
        if let Some(stream) = self
            .streams
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
        {
            return Some(stream.clone());
        }
        if !crate::license::seats_available(self.count() as u32) {
            return None;
        }
        let mut streams = self.streams.write().unwrap_or_else(|e| e.into_inner());
        let created = !streams.contains_key(&key);
        let stream = streams
            .entry(key)
            .or_insert_with(|| Arc::new(Stream::new(app, name, self.gop_cache)))
            .clone();
        drop(streams);
        if created {
            crate::events::publish("stream.created", &format!("{app}/{name}"));
        }
        Some(stream)
    }

    pub fn get_or_create(&self, app: &str, name: &str) -> Arc<Stream> {
        self.try_get_or_create(app, name)
            .expect("stream seat limit exceeded")
    }

    pub fn remove(&self, app: &str, name: &str) -> bool {
        let removed = self
            .streams
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&format!("{app}/{name}"))
            .is_some();
        if removed {
            crate::events::publish("stream.removed", &format!("{app}/{name}"));
        }
        removed
    }

    pub fn get(&self, app: &str, name: &str) -> Option<Arc<Stream>> {
        self.streams
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&format!("{app}/{name}"))
            .cloned()
    }

    pub fn list(&self) -> Vec<Value> {
        let mut out: Vec<Value> = self
            .streams
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|s| s.json())
            .collect();
        out.sort_by(|a, b| a["key"].as_str().cmp(&b["key"].as_str()));
        out
    }

    pub fn count(&self) -> usize {
        self.streams.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn subscriber_count(&self) -> usize {
        self.streams
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|s| s.json()["subscribers"].as_u64().unwrap_or(0) as usize)
            .sum()
    }
}

impl m7s_sdk::TagSink for Stream {
    fn push_tag(&self, tag_type: u8, timestamp: u32, data: Vec<u8>) {
        self.push(Arc::new(FlvTag::new(tag_type, timestamp, data)));
    }

    fn leave(&self) {
        Stream::leave(self);
    }
}

struct StreamTagReader(crate::dispatcher::DispatchReader);

impl m7s_sdk::TagReader for StreamTagReader {
    fn try_next(&mut self) -> Option<m7s_sdk::Tag> {
        let tag = self.0.try_next()?;
        Some(m7s_sdk::Tag {
            tag_type: tag.tag_type,
            timestamp: tag.timestamp,
            data: tag.data.clone(),
        })
    }
}

impl m7s_sdk::TagStream for Stream {
    fn tag_reader(&self) -> Box<dyn m7s_sdk::TagReader> {
        Box::new(StreamTagReader(self.dispatch_reader()))
    }

    fn tap(&self) -> tokio::sync::broadcast::Receiver<m7s_sdk::SharedTag> {
        self.tap_tx.subscribe()
    }

    fn snapshot(&self) -> Vec<m7s_sdk::Tag> {
        self.cache_snapshot()
            .iter()
            .map(|t| m7s_sdk::Tag {
                tag_type: t.tag_type,
                timestamp: t.timestamp,
                data: t.data.clone(),
            })
            .collect()
    }
    fn metadata(&self) -> Option<m7s_sdk::Tag> {
        Stream::metadata(self).map(|t| m7s_sdk::Tag {
            tag_type: t.tag_type,
            timestamp: t.timestamp,
            data: t.data.clone(),
        })
    }
}

impl m7s_sdk::StreamApi for StreamManager {
    fn stream(&self, app: &str, name: &str) -> Option<Arc<dyn m7s_sdk::TagStream>> {
        self.try_get_or_create(app, name)
            .map(|s| s as Arc<dyn m7s_sdk::TagStream>)
    }

    fn get(&self, app: &str, name: &str) -> Option<Arc<dyn m7s_sdk::TagStream>> {
        StreamManager::get(self, app, name).map(|s| s as Arc<dyn m7s_sdk::TagStream>)
    }

    fn remove(&self, app: &str, name: &str) -> bool {
        StreamManager::remove(self, app, name)
    }

    fn list(&self) -> Vec<m7s_sdk::StreamInfo> {
        StreamManager::list(self)
            .into_iter()
            .map(|s| m7s_sdk::StreamInfo {
                app: s["app"].as_str().unwrap_or("").to_string(),
                name: s["name"].as_str().unwrap_or("").to_string(),
                key: s["key"].as_str().unwrap_or("").to_string(),
                subscribers: s["subscribers"].as_u64().unwrap_or(0) as usize,
                bytes: s["bytes"].as_u64().unwrap_or(0),
            })
            .collect()
    }

    fn count(&self) -> usize {
        StreamManager::count(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_and_broadcast() {
        let mgr = StreamManager::new(2);
        let stream = mgr.get_or_create("live", "test");
        let (cache, mut rx) = stream.subscribe();
        assert!(cache.is_empty());
        stream.push(Arc::new(FlvTag::new(9, 0, vec![1])));
        stream.push(Arc::new(FlvTag::new(9, 1, vec![2])));
        stream.push(Arc::new(FlvTag::new(9, 2, vec![3])));
        assert_eq!(stream.json()["cached_tags"], 2);
        let got = rx.try_recv().unwrap();
        assert_eq!(got.timestamp, 0);
        stream.leave();
        assert_eq!(stream.json()["subscribers"], 0);
    }

    #[test]
    fn tag_stream_reader() {
        use m7s_sdk::{TagSink, TagStream};
        let mgr = StreamManager::new(10);
        let stream = mgr.get_or_create("live", "ts");
        let mut reader = stream.tag_reader();
        stream.push_tag(9, 42, vec![0x17, 1, 2]);
        let tag = reader.try_next().unwrap();
        assert_eq!(tag.tag_type, 9);
        assert_eq!(tag.timestamp, 42);
        assert_eq!(tag.data, vec![0x17, 1, 2]);
        stream.leave();
    }

    #[test]
    fn stream_api_lists_sessions() {
        use m7s_sdk::TagSink;
        let mgr = StreamManager::new(10);
        let stream = mgr.get_or_create("live", "api");
        stream.push_tag(9, 1, vec![1]);
        let list = <StreamManager as m7s_sdk::StreamApi>::list(&mgr);
        assert_eq!(<StreamManager as m7s_sdk::StreamApi>::count(&mgr), 1);
        assert_eq!(list[0].key, "live/api");
        assert_eq!(list[0].bytes, 12);
        stream.leave();
    }

    #[test]
    fn tag_stream_tap() {
        use m7s_sdk::{TagSink, TagStream};
        let mgr = StreamManager::new(10);
        let stream = mgr.get_or_create("live", "tap");
        let mut rx = stream.tap();
        stream.push_tag(9, 7, vec![1, 2]);
        let tag = rx.try_recv().unwrap();
        assert_eq!(tag.timestamp, 7);
        assert_eq!(tag.data, vec![1, 2]);
        stream.leave();
    }

    #[test]
    fn ring_shares_arc() {
        let mgr = StreamManager::new(10);
        let stream = mgr.get_or_create("live", "ring");
        let mut reader = stream.ring_reader();
        let tag = Arc::new(FlvTag::new(9, 10, vec![7]));
        stream.push(tag.clone());
        let got = reader.try_next().unwrap();
        assert!(Arc::ptr_eq(&tag, &got));
    }

    #[test]
    fn timestamp_reset_is_normalized() {
        let mgr = StreamManager::new(10);
        let stream = mgr.get_or_create("live", "test");
        stream.push(Arc::new(FlvTag::new(9, 5000, vec![1])));
        stream.push(Arc::new(FlvTag::new(9, 0, vec![2])));
        let (cache, _rx) = stream.subscribe();
        assert_eq!(cache.last().unwrap().timestamp, 5040);
        stream.leave();
    }
}

