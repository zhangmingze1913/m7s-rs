//! HLS segmenter: FLV tags -> MPEG-TS segments + m3u8 playlist.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use crate::config::HlsConfig;
use crate::stream::StreamManager;
use m7s_sdk::TagStream;

#[derive(Debug, Clone)]
pub struct Segment {
    pub seq: u64,
    pub duration: f64,
    pub data: Vec<u8>,
}

pub struct HlsStream {
    segments: RwLock<VecDeque<Segment>>,
    running: AtomicBool,
    max_segments: usize,
    encrypt: bool,
    key: [u8; 16],
}

impl HlsStream {
    fn new(max_segments: usize, encrypt: bool, key_hex: &str) -> Self {
        let mut key = [0u8; 16];
        if key_hex.len() >= 32 {
            for (i, slot) in key.iter_mut().enumerate() {
                *slot = u8::from_str_radix(&key_hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
            }
        }
        Self {
            segments: RwLock::new(VecDeque::new()),
            running: AtomicBool::new(false),
            max_segments: max_segments.max(1),
            encrypt,
            key,
        }
    }

    fn push(&self, mut segment: Segment) {
        if self.encrypt {
            segment.data = encrypt_segment(&segment.data, &self.key, segment.seq);
        }
        let mut segments = self.segments.write().unwrap_or_else(|e| e.into_inner());
        segments.push_back(segment);
        while segments.len() > self.max_segments {
            segments.pop_front();
        }
    }

    pub fn playlist(&self, target_secs: u64) -> String {
        let segments = self.segments.read().unwrap_or_else(|e| e.into_inner());
        let items: Vec<m7s_plugin_hls::PlaylistSegment> = segments
            .iter()
            .map(|s| m7s_plugin_hls::PlaylistSegment { seq: s.seq, duration: s.duration })
            .collect();
        let mut out = m7s_plugin_hls::playlist(&items, target_secs);
        if self.encrypt {
            out = out.replacen(
                "#EXTM3U\n",
                "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n",
                1,
            );
        }
        out
    }

    pub fn key(&self) -> [u8; 16] {
        self.key
    }

    pub fn segment(&self, seq: u64) -> Option<Vec<u8>> {
        self.segments
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|s| s.seq == seq)
            .map(|s| s.data.clone())
    }

    pub fn segment_count(&self) -> usize {
        self.segments
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }
}

pub struct HlsManager {
    streams: Arc<StreamManager>,
    cfg: std::sync::RwLock<HlsConfig>,
    playlists: RwLock<HashMap<String, Arc<HlsStream>>>,
}

impl HlsManager {
    pub fn new(streams: Arc<StreamManager>, cfg: HlsConfig) -> Arc<Self> {
        Arc::new(Self {
            streams,
            cfg: std::sync::RwLock::new(cfg),
            playlists: RwLock::new(HashMap::new()),
        })
    }

    /// Apply a new HLS config at runtime. Existing playlists keep running;
    /// new/restarted playlists use the new segment/encryption settings.
    pub fn reload(&self, cfg: HlsConfig) {
        *self.cfg.write().unwrap_or_else(|e| e.into_inner()) = cfg;
        self.playlists.write().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub fn encrypt(&self) -> bool {
        self.cfg.read().unwrap_or_else(|e| e.into_inner()).encrypt
    }

    pub fn key_bytes(&self) -> [u8; 16] {
        parse_key(&self.cfg.read().unwrap_or_else(|e| e.into_inner()).key)
    }

    pub fn ensure(&self, app: &str, name: &str) -> Option<Arc<HlsStream>> {
        if !self.cfg.read().unwrap_or_else(|e| e.into_inner()).enabled {
            return None;
        }
        let key = format!("{app}/{name}");
        let hls = {
            let mut map = self.playlists.write().unwrap_or_else(|e| e.into_inner());
            map.entry(key.clone())
                .or_insert_with(|| {
                let cfg = self.cfg.read().unwrap_or_else(|e| e.into_inner());
                Arc::new(HlsStream::new(cfg.max_segments, cfg.encrypt, &cfg.key))
            })
                .clone()
        };
        if hls
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let Some(stream) = self.streams.get(app, name) else {
                hls.running.store(false, Ordering::Release);
                return Some(hls);
            };
            let target = self.cfg.read().unwrap_or_else(|e| e.into_inner()).segment_secs;
            let hls_task = hls.clone();
            tokio::spawn(async move {
                segment_loop(stream, hls_task.clone(), target).await;
                hls_task.running.store(false, Ordering::Release);
            });
        }
        Some(hls)
    }
}

async fn segment_loop(stream: Arc<crate::stream::Stream>, hls: Arc<HlsStream>, target_secs: u64) {
    let mut reader = stream.tag_reader();
    let mut segmenter = m7s_plugin_hls::Segmenter::new(target_secs.max(1));
    loop {
        let mut got = false;
        while let Some(tag) = reader.try_next() {
            got = true;
            let mut outputs = Vec::new();
            segmenter.push(&tag, &mut outputs);
            for output in outputs {
                hls.push(Segment {
                    seq: output.seq,
                    duration: output.duration,
                    data: output.data,
                });
            }
        }
        if !got {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use m7s_plugin_hls::adts_frame;
    use crate::flv::FlvTag;
    use super::*;
    use crate::stream::StreamManager;
    use std::sync::Arc;

    #[test]
    fn adts_header() {
        let frame = adts_frame(&[0x12, 0x10], &[1, 2, 3, 4]);
        assert_eq!(frame[0], 0xff);
        assert_eq!(frame[1], 0xf1);
        assert_eq!(frame.len(), 11);
    }

    #[test]
    fn segments_h264_keyframes() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let streams = StreamManager::new(100);
            let stream = streams.get_or_create("live", "test");
            let config = vec![
                0x01, 0x42, 0x00, 0x1e, 0xff, 0xe1, 0x00, 0x04, 0x67, 0x42, 0x00, 0x1e, 0x01, 0x00,
                0x03, 0x68, 0xce, 0x06, 0xe2,
            ];
            let mut seq = vec![0x17, 0, 0, 0, 0];
            seq.extend_from_slice(&config);
            stream.push(Arc::new(FlvTag::new(9, 0, seq)));
            for (ts, payload) in [(0u32, vec![0x65, 1, 2, 3]), (1500, vec![0x65, 4, 5, 6])] {
                let mut data = vec![0x17, 1, 0, 0, 0, 0, 0, 0, 0x04];
                data.extend_from_slice(&payload);
                stream.push(Arc::new(FlvTag::new(9, ts, data)));
            }
            let hls = HlsManager::new(
                streams.clone(),
                HlsConfig {
                    enabled: true,
                    segment_secs: 1,
                    max_segments: 4,
                    ..Default::default()
                },
            );
            let playlist = hls.ensure("live", "test").unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let text = playlist.playlist(1);
            assert!(text.contains("#EXTM3U"), "{text}");
            assert!(text.contains("seg-0.ts"), "{text}");
            let segment = playlist.segment(0).expect("segment");
            assert_eq!(segment[0], 0x47);
        });
    }
}

pub(crate) fn encrypt_segment(data: &[u8], key: &[u8; 16], seq: u64) -> Vec<u8> {
    use aes::cipher::block_padding::Pkcs7;
    use aes::cipher::{BlockModeEncrypt, KeyIvInit};
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&seq.to_be_bytes());
    let enc = cbc::Encryptor::<aes::Aes128>::new(key.into(), (&iv).into());
    enc.encrypt_padded_vec::<Pkcs7>(data)
}

#[cfg(test)]
mod encrypt_tests {
    use super::*;
    use aes::cipher::block_padding::Pkcs7;
    use aes::cipher::{BlockModeDecrypt, KeyIvInit};

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = [7u8; 16];
        let plain = b"hello hls segment";
        let cipher = encrypt_segment(plain, &key, 3);
        assert_ne!(&cipher[..], &plain[..]);
        let mut iv = [0u8; 16];
        iv[8..].copy_from_slice(&3u64.to_be_bytes());
        let dec = cbc::Decryptor::<aes::Aes128>::new((&key).into(), (&iv).into());
        let out = dec.decrypt_padded_vec::<Pkcs7>(&cipher).unwrap();
        assert_eq!(out, plain);
    }

    #[test]
    fn playlist_has_key() {
        let h = HlsStream::new(6, true, &"11".repeat(16));
        assert!(h.playlist(4).contains("EXT-X-KEY"));
        let h2 = HlsStream::new(6, false, "");
        assert!(!h2.playlist(4).contains("EXT-X-KEY"));
    }
}

fn parse_key(key_hex: &str) -> [u8; 16] {
    let mut key = [0u8; 16];
    if key_hex.len() >= 32 {
        for (i, slot) in key.iter_mut().enumerate() {
            *slot = u8::from_str_radix(&key_hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
        }
    }
    key
}
