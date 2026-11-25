use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn flv_redirects_to_shard_owner() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            let cfg = ClusterConfig {
                enabled: false,
                node_id: "n1".to_string(),
                advertise: "127.0.0.1:8180".to_string(),
                seeds: vec!["10.0.0.2:8180".to_string(), "10.0.0.3:8180".to_string()],
                shard_redirect: true,
                ..ClusterConfig::default()
            };
            let cluster = Cluster::new(cfg, StreamManager::new(10));
            let state = AppState {
                streams: streams.clone(),
                cluster,
                hls: HlsManager::new(streams, HlsConfig::default()),
                node_id: "n1".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            let mut redirected = false;
            for i in 0..200 {
                let text = request(port, &format!("/flv/live/s{i}.flv")).await;
                if text.starts_with("HTTP/1.1 307") {
                    assert!(text.contains("Location: http://10.0.0."), "{text}");
                    redirected = true;
                    break;
                }
            }
            assert!(redirected, "no shard redirect");
        });
}

async fn request(port: u16, path: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: t\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf).to_string()
}
