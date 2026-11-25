use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::flv::FlvTag;
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn fmp4_playlist_blocking_reload() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            let stream = streams.get_or_create("live", "fm");
            let cfg = vec![0x17, 0, 0, 0, 0, 1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4, 0x68, 1, 2, 3];
            stream.push(Arc::new(FlvTag::new(9, 0, cfg)));
            stream.push(Arc::new(FlvTag::new(9, 0, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 0x65, 1, 2])));
            let state = AppState {
                streams: streams.clone(),
                cluster: Cluster::new(
                    ClusterConfig { enabled: false, ..ClusterConfig::default() },
                    StreamManager::new(10),
                ),
                hls: HlsManager::new(streams, HlsConfig::default()),
                node_id: "t".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            let (status, body) = get(port, "/fmp4/live/fm/index.m3u8?_HLS_msn=0").await;
            assert_eq!(status, 200);
            let text = String::from_utf8_lossy(&body);
            assert!(text.contains("#EXT-X-PART-INF"), "{text}");
            assert!(text.contains("#EXT-X-PART:"), "{text}");
            let (status, part) = get(port, "/fmp4/live/fm/part-0-0.m4s").await;
            assert_eq!(status, 200);
            assert!(part.windows(4).any(|w| w == b"moof"));
            let (status, _) = get(port, "/fmp4/live/fm/index.m3u8?_HLS_msn=0&_HLS_part=0").await;
            assert_eq!(status, 200);
            let (status, part) = get_range(port, "/fmp4/live/fm/seg-0.m4s", "bytes=0-9").await;
            assert_eq!(status, 206);
            assert_eq!(part.len(), 10);
        });
}


async fn get(port: u16, path: &str) -> (u16, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: t\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    let status = text.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let body = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| buf[p + 4..].to_vec())
        .unwrap_or_default();
    (status, body)
}

#[test]
fn fmp4_audio_only() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            let stream = streams.get_or_create("live", "fma");
            stream.push(Arc::new(FlvTag::new(8, 0, vec![0xAF, 0x00, 0x12, 0x10])));
            stream.push(Arc::new(FlvTag::new(8, 0, vec![0xAF, 0x01, 1, 2, 3, 4])));
            stream.push(Arc::new(FlvTag::new(8, 23, vec![0xAF, 0x01, 5, 6, 7])));
            let state = AppState {
                streams: streams.clone(),
                cluster: Cluster::new(ClusterConfig { enabled: false, ..ClusterConfig::default() }, StreamManager::new(10)),
                hls: HlsManager::new(streams, HlsConfig::default()),
                node_id: "t".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            let (status, body) = get(port, "/fmp4/live/fma/aud.m3u8").await;
            assert_eq!(status, 200);
            let text = String::from_utf8_lossy(&body);
            assert!(text.contains("aud_init.mp4"), "{text}");
            assert!(text.contains("aud-0.m4s"), "{text}");

            let (status, init) = get(port, "/fmp4/live/fma/aud_init.mp4").await;
            assert_eq!(status, 200);
            assert!(init.windows(4).any(|w| w == b"moov"));
            assert!(init.windows(4).any(|w| w == b"mp4a"));

            let (status, seg) = get(port, "/fmp4/live/fma/aud-0.m4s").await;
            assert_eq!(status, 200);
            assert!(seg.windows(4).any(|w| w == b"moof"));
        });
}

#[test]
fn fmp4_hls_part_blocking() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            let stream = streams.get_or_create("live", "fmblock");
            let cfg = vec![0x17, 0, 0, 0, 0, 1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4, 0x68, 1, 2, 3];
            stream.push(Arc::new(FlvTag::new(9, 0, cfg)));
            stream.push(Arc::new(FlvTag::new(9, 0, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 0x65, 1, 2])));
            let state = AppState {
                streams: streams.clone(),
                cluster: Cluster::new(ClusterConfig { enabled: false, ..ClusterConfig::default() }, StreamManager::new(10)),
                hls: HlsManager::new(streams, HlsConfig::default()),
                node_id: "t".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            let pending = tokio::spawn(async move {
                get(port, "/fmp4/live/fmblock/index.m3u8?_HLS_msn=1&_HLS_part=0").await
            });
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            stream.push(Arc::new(FlvTag::new(9, 3000, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 0x65, 3, 4])));
            let (status, body) = pending.await.unwrap();
            assert_eq!(status, 200);
            let text = String::from_utf8_lossy(&body);
            assert!(text.contains("seg-1.m4s"), "{text}");
            assert!(text.contains("#EXT-X-PRELOAD-HINT"), "{text}");
        });
}

async fn get_range(port: u16, path: &str, range: &str) -> (u16, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: t\r\nRange: {range}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    let status = text.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let body = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| buf[p + 4..].to_vec())
        .unwrap_or_default();
    (status, body)
}
