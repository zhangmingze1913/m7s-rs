use std::sync::Arc;
use std::time::Duration;

use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::flv::FlvTag;
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn hls_playlist_and_segment_over_http() {
    runtime().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
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
        hls.ensure("live", "test");
        tokio::time::sleep(Duration::from_millis(300)).await;
        let state = AppState {
            streams,
            cluster: Cluster::new(
                ClusterConfig {
                    enabled: false,
                    ..ClusterConfig::default()
                },
                StreamManager::new(10),
            ),
            hls,
            node_id: "test".to_string(),
            api_token: String::new(),
        };
        tokio::spawn(http_run(listener, state));

        let (status, body) = http_get(port, "/hls/live/test/index.m3u8").await;
        assert_eq!(status, 200);
        let playlist = String::from_utf8_lossy(&body);
        assert!(playlist.contains("#EXTM3U"), "{playlist}");
        assert!(playlist.contains("seg-0.ts"), "{playlist}");

        let (status, segment) = http_get(port, "/hls/live/test/seg-0.ts").await;
        assert_eq!(status, 200);
        assert!(segment.len() > 188);
        assert_eq!(segment[0], 0x47);
    });
}

async fn http_get(port: u16, path: &str) -> (u16, Vec<u8>) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream
        .write_all(format!("GET {path} HTTP/1.1\r\nHost: test\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let text = String::from_utf8_lossy(&buf);
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| buf[p + 4..].to_vec())
        .unwrap_or_default();
    (status, body)
}

#[test]
fn hls_aes128_playlist_and_key() {
    runtime().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let streams = StreamManager::new(100);
        let stream = streams.get_or_create("live", "enc");
        let config = vec![0x01, 0x42, 0x00, 0x1e, 0xff, 0xe1, 0x00, 0x04, 0x67, 0x42, 0x00, 0x1e, 0x01, 0x00, 0x03, 0x68, 0xce, 0x06, 0xe2];
        let mut seq = vec![0x17, 0, 0, 0, 0];
        seq.extend_from_slice(&config);
        stream.push(Arc::new(FlvTag::new(9, 0, seq)));
        stream.push(Arc::new(FlvTag::new(9, 0, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 4, 0x65, 1, 2, 3])));
        stream.push(Arc::new(FlvTag::new(9, 1500, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 4, 0x65, 4, 5, 6])));
        let key = "000102030405060708090a0b0c0d0e0f".to_string();
        let hls = HlsManager::new(
            streams.clone(),
            HlsConfig {
                enabled: true,
                segment_secs: 1,
                max_segments: 4,
                encrypt: true,
                key: key.clone(),
            },
        );
        hls.ensure("live", "enc");
        tokio::time::sleep(Duration::from_millis(300)).await;
        let state = AppState {
            streams,
            cluster: Cluster::new(ClusterConfig { enabled: false, ..ClusterConfig::default() }, StreamManager::new(10)),
            hls,
            node_id: "test".to_string(),
            api_token: String::new(),
        };
        tokio::spawn(http_run(listener, state));

        let (status, body) = http_get(port, "/hls/live/enc/index.m3u8").await;
        assert_eq!(status, 200);
        let playlist = String::from_utf8_lossy(&body);
        assert!(playlist.contains("EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\""), "{playlist}");

        let (status, key_body) = http_get(port, "/hls/live/enc/key.bin").await;
        assert_eq!(status, 200);
        assert_eq!(key_body.len(), 16);
        assert_eq!(key_body, (0u8..16).collect::<Vec<_>>());

        let (status, segment) = http_get(port, "/hls/live/enc/seg-0.ts").await;
        assert_eq!(status, 200);
        assert!(!segment.is_empty());
        assert_ne!(segment[0], 0x47, "segment should be encrypted");
    });
}

#[test]
fn hls_audio_only_ts() {
    runtime().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let streams = StreamManager::new(100);
        let stream = streams.get_or_create("live", "aud");
        stream.push(Arc::new(FlvTag::new(8, 0, vec![0xAF, 0x00, 0x12, 0x10])));
        for ts in [0u32, 500, 1000] {
            stream.push(Arc::new(FlvTag::new(8, ts, vec![0xAF, 0x01, 1, 2, 3, 4])));
        }
        let hls = HlsManager::new(
            streams.clone(),
            HlsConfig { enabled: true, segment_secs: 1, max_segments: 4, ..Default::default() },
        );
        hls.ensure("live", "aud");
        tokio::time::sleep(Duration::from_millis(300)).await;
        let state = AppState {
            streams,
            cluster: Cluster::new(ClusterConfig { enabled: false, ..ClusterConfig::default() }, StreamManager::new(10)),
            hls,
            node_id: "test".to_string(),
            api_token: String::new(),
        };
        tokio::spawn(http_run(listener, state));

        let (status, body) = http_get(port, "/hls/live/aud/index.m3u8").await;
        assert_eq!(status, 200);
        assert!(String::from_utf8_lossy(&body).contains("seg-0.ts"));
        let (status, segment) = http_get(port, "/hls/live/aud/seg-0.ts").await;
        assert_eq!(status, 200);
        assert!(segment.len() > 188);
        assert_eq!(segment[0], 0x47);
    });
}
