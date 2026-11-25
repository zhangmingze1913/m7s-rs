use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn cluster_allocate_shards_streams() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            let cfg = ClusterConfig {
                enabled: true,
                node_id: "n1".to_string(),
                advertise: "127.0.0.1:8180".to_string(),
                seeds: vec!["10.0.0.2:8180".to_string(), "10.0.0.3:8180".to_string()],
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

            let mut local = false;
            let mut remote = false;
            for i in 0..200 {
                let (status, body) = get(port, &format!("/cluster/api/allocate/play?app=live&stream=s{i}")).await;
                assert_eq!(status, 200);
                let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let redirect = v["data"]["redirect"].as_bool().unwrap_or(false);
                let server = v["data"]["server_id"].as_str().unwrap_or("");
                assert!(server == "n1" || server.starts_with("10.0.0."), "{v}");
                if redirect { remote = true } else { local = true }
                if local && remote { break }
            }
            assert!(local && remote, "expected both local and remote shards");
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
