use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn http_rejects_over_connection_limit() {
    unsafe {
        std::env::set_var("M7S_MAX_CONNECTIONS", "1");
    }
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
                node_id: "limit".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            let guard = m7s_sdk::connections::try_acquire().unwrap();
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(b"GET /api/plugins HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf).await;
            assert!(buf.is_empty(), "connection should be rejected");
            drop(guard);

            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(b"GET /api/plugins HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut buf = Vec::new();
            s.read_to_end(&mut buf).await.unwrap();
            assert!(String::from_utf8_lossy(&buf).contains("200 OK"));
        });
    unsafe {
        std::env::remove_var("M7S_MAX_CONNECTIONS");
    }
}
