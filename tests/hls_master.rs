use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn hls_master_lists_variants() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            streams.get_or_create("live", "main");
            streams.get_or_create("live", "main_1500");
            streams.get_or_create("live", "other_9999");
            let state = AppState {
                streams: streams.clone(),
                cluster: Cluster::new(ClusterConfig { enabled: false, ..ClusterConfig::default() }, StreamManager::new(10)),
                hls: HlsManager::new(streams, HlsConfig::default()),
                node_id: "hls".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            let (status, body) = get(port, "/hls/live/main/master.m3u8").await;
            assert_eq!(status, 200);
            let text = String::from_utf8_lossy(&body);
            assert_eq!(text.matches("#EXT-X-STREAM-INF").count(), 2, "{text}");
            assert!(text.contains("BANDWIDTH=800000"), "{text}");
            assert!(text.contains("BANDWIDTH=1500000"), "{text}");
            assert!(text.contains("../main_1500/index.m3u8"), "{text}");
            assert!(!text.contains("other_9999"), "{text}");

            let (status, _) = get(port, "/hls/live/missing/master.m3u8").await;
            assert_eq!(status, 404);
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
