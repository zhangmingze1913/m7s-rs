use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn alerts_endpoint() {
    unsafe {
        std::env::set_var("M7S_ALERT_STREAMS", "1");
        std::env::set_var("M7S_ALERT_DROPPED", "1");
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            streams.get_or_create("live", "alert");
            let state = AppState {
                streams: streams.clone(),
                cluster: Cluster::new(ClusterConfig { enabled: false, ..ClusterConfig::default() }, StreamManager::new(10)),
                hls: HlsManager::new(streams, HlsConfig::default()),
                node_id: "alerts".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(b"GET /api/alerts HTTP/1.1\r\nHost: t\r\n\r\n").await.unwrap();
            let mut buf = Vec::new();
            s.read_to_end(&mut buf).await.unwrap();
            let text = String::from_utf8_lossy(&buf);
            assert!(text.contains("streams.high"), "{text}");
        });
}

#[test]
fn alerts_sse_stream() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(5);
            let state = AppState {
                streams: streams.clone(),
                cluster: Cluster::new(ClusterConfig { enabled: false, ..ClusterConfig::default() }, StreamManager::new(5)),
                hls: HlsManager::new(streams, HlsConfig::default()),
                node_id: "sse".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(b"GET /api/alerts/stream?count=3 HTTP/1.1\r\nHost: t\r\n\r\n").await.unwrap();
            let mut buf = Vec::new();
            s.read_to_end(&mut buf).await.unwrap();
            let text = String::from_utf8_lossy(&buf);
            assert!(text.contains("text/event-stream"), "{text}");
            assert_eq!(text.matches("data: ").count(), 3, "{text}");
        });
}
