use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn events_sse_receives_stream_created() {
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
                hls: HlsManager::new(streams.clone(), HlsConfig::default()),
                node_id: "evt".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(b"GET /api/events/stream?count=1 HTTP/1.1\r\nHost: t\r\n\r\n")
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            streams.get_or_create("live", "sse");
            let mut buf = Vec::new();
            tokio::time::timeout(std::time::Duration::from_secs(5), s.read_to_end(&mut buf))
                .await
                .unwrap()
                .unwrap();
            let text = String::from_utf8_lossy(&buf);
            assert!(text.contains("stream.created"), "{text}");
            assert!(text.contains("live/sse"), "{text}");
        });
}
