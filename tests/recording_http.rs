use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn recording_list_and_playback() {
    let dir = std::env::temp_dir().join(format!("m7s-rec-http-{}", std::process::id()));
    let mp4 = dir.join("mp4");
    std::fs::create_dir_all(&mp4).unwrap();
    std::fs::write(dir.join("a.flv"), b"flv-data").unwrap();
    std::fs::write(mp4.join("b.mp4"), b"mp4-data").unwrap();
    unsafe {
        std::env::set_var("M7S_RECORD_DIR", &dir);
        std::env::set_var("M7S_MP4_DIR", &mp4);
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            let state = AppState {
                streams: streams.clone(),
                cluster: Cluster::new(ClusterConfig { enabled: false, ..ClusterConfig::default() }, StreamManager::new(10)),
                hls: HlsManager::new(streams, HlsConfig::default()),
                node_id: "rec".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            let (_, body) = get(port, "/api/recordings").await;
            assert!(String::from_utf8_lossy(&body).contains("a.flv"));
            let (_, body) = get(port, "/recordings/a.flv").await;
            assert_eq!(body, b"flv-data");
            let (_, body) = get(port, "/api/meeting/recordings").await;
            assert!(String::from_utf8_lossy(&body).contains("b.mp4"));
            let (_, body) = get(port, "/api/meeting/playback?file=b.mp4").await;
            assert_eq!(body, b"mp4-data");
            let (status, _) = get(port, "/recordings/../a.flv").await;
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
