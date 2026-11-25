//! Unified media frame model (P1.1).
//!
//! This is the first step of the data-plane refactor: protocols still carry
//! `FlvTag`, but new code can convert to a track-aware frame. Payload is an
//! `Arc<[u8]>` so downstream muxers can share it without copying.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Notify;

use crate::flv::FlvTag;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
    Metadata,
}

impl TrackKind {
    pub fn from_tag(tag: &FlvTag) -> Self {
        match tag.tag_type {
            9 => Self::Video,
            8 => Self::Audio,
            _ => Self::Metadata,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFrame {
    pub track: TrackKind,
    pub sequence: u64,
    pub timestamp_ms: u32,
    pub keyframe: bool,
    pub data: Arc<[u8]>,
}

/// Bounded per-track frame ring. Publishing is one write; each subscriber
/// advances its own cursor, so fan-out no longer sends one message per viewer.
pub struct MediaRing {
    frames: RwLock<VecDeque<Arc<MediaFrame>>>,
    capacity: usize,
    next_seq: AtomicU64,
    notify: Arc<Notify>,
}

impl MediaRing {
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            frames: RwLock::new(VecDeque::with_capacity(capacity.max(1))),
            capacity: capacity.max(1),
            next_seq: AtomicU64::new(0),
            notify: Arc::new(Notify::new()),
        })
    }

    pub fn push(&self, mut frame: MediaFrame) {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        frame.sequence = seq;
        let mut frames = self.frames.write().unwrap_or_else(|e| e.into_inner());
        frames.push_back(Arc::new(frame));
        while frames.len() > self.capacity {
            frames.pop_front();
        }
        drop(frames);
        self.notify.notify_waiters();
    }

    pub fn subscribe(self: &Arc<Self>) -> MediaSubscriber {
        let cursor = self
            .frames
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .front()
            .map(|f| f.sequence)
            .unwrap_or(0);
        MediaSubscriber {
            ring: self.clone(),
            cursor,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn len(&self) -> usize {
        self.frames.read().unwrap_or_else(|e| e.into_inner()).len()
    }
}

pub struct MediaSubscriber {
    ring: Arc<MediaRing>,
    cursor: u64,
}

impl MediaSubscriber {
    pub async fn next(&mut self) -> Arc<MediaFrame> {
        loop {
            let notified = self.ring.notify.notified();
            let next = {
                let frames = self.ring.frames.read().unwrap_or_else(|e| e.into_inner());
                if let Some(front) = frames.front() {
                    if self.cursor < front.sequence {
                        self.cursor = frames
                            .iter()
                            .find(|f| f.keyframe)
                            .map(|f| f.sequence)
                            .unwrap_or(front.sequence);
                    }
                }
                frames
                    .iter()
                    .find(|f| f.sequence >= self.cursor)
                    .inspect(|f| {
                        self.cursor = f.sequence + 1;
                    })
                    .cloned()
            };
            if let Some(frame) = next {
                return frame;
            }
            notified.await;
        }
    }
}
impl MediaFrame {
    pub fn from_flv(tag: &FlvTag, sequence: u64) -> Self {
        let track = TrackKind::from_tag(tag);
        let keyframe = match track {
            TrackKind::Video => tag.data.first().is_some_and(|b| (b >> 4) == 1),
            _ => false,
        };
        Self {
            track,
            sequence,
            timestamp_ms: tag.timestamp,
            keyframe,
            data: Arc::from(tag.data.clone()),
        }
    }

    pub fn is_video(&self) -> bool {
        self.track == TrackKind::Video
    }

    pub fn is_audio(&self) -> bool {
        self.track == TrackKind::Audio
    }

    pub fn is_metadata(&self) -> bool {
        self.track == TrackKind::Metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_keyframe() {
        let tag = FlvTag::new(9, 40, vec![0x17, 1, 2, 3]);
        let frame = MediaFrame::from_flv(&tag, 7);
        assert_eq!(frame.track, TrackKind::Video);
        assert!(frame.is_video());
        assert!(frame.keyframe);
        assert_eq!(frame.sequence, 7);
        assert_eq!(&*frame.data, &[0x17, 1, 2, 3]);
    }

    #[test]
    fn audio_and_metadata() {
        let audio = MediaFrame::from_flv(&FlvTag::new(8, 10, vec![0xaf, 1]), 1);
        assert!(audio.is_audio());
        assert!(!audio.keyframe);
        let meta = MediaFrame::from_flv(&FlvTag::new(18, 0, vec![0x02]), 2);
        assert!(meta.is_metadata());
    }
    #[test]
    fn ring_fanout() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let ring = MediaRing::new(4);
            let mut a = ring.subscribe();
            let mut b = ring.subscribe();
            ring.push(MediaFrame::from_flv(&FlvTag::new(9, 0, vec![0x17, 1]), 0));
            ring.push(MediaFrame::from_flv(&FlvTag::new(9, 40, vec![0x27, 1]), 0));
            assert_eq!(a.next().await.sequence, 0);
            assert_eq!(b.next().await.sequence, 0);
            assert_eq!(a.next().await.sequence, 1);
            assert_eq!(b.next().await.sequence, 1);
        });
    }

    #[test]
    fn ring_lag_seeks_keyframe() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let ring = MediaRing::new(2);
            let mut sub = ring.subscribe();
            ring.push(MediaFrame::from_flv(&FlvTag::new(9, 0, vec![0x17, 1]), 0));
            ring.push(MediaFrame::from_flv(&FlvTag::new(9, 40, vec![0x27, 1]), 0));
            ring.push(MediaFrame::from_flv(&FlvTag::new(9, 80, vec![0x17, 1]), 0));
            let frame = sub.next().await;
            assert_eq!(frame.sequence, 2);
            assert!(frame.keyframe);
        });
    }
}
