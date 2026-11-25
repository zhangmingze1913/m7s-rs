use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use m7s_rs::flv::FlvTag;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn plugin_matrix_26_and_toggle() {
    let cfg = std::env::temp_dir().join(format!("m7s-plugins-{}.json", std::process::id()));
    unsafe { std::env::set_var("M7S_PLUGIN_CONFIG", &cfg) };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            let stream = streams.get_or_create("live", "hls");
            stream.push(Arc::new(FlvTag::new(9, 0, vec![0x17, 0, 0, 0, 0, 1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4, 0x68, 1, 2, 3])));
            stream.push(Arc::new(FlvTag::new(9, 0, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 4, 0x65, 1, 2, 3])));
            stream.push(Arc::new(FlvTag::new(9, 1500, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 4, 0x65, 4, 5, 6])));
            let hls = HlsManager::new(streams.clone(), HlsConfig::default());
            hls.ensure("live", "hls");
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let state = AppState {
                streams: streams.clone(),
                cluster: Cluster::new(
                    ClusterConfig { enabled: false, ..ClusterConfig::default() },
                    StreamManager::new(10),
                ),
                hls,
                node_id: "plugins".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            let (_, body) = get(port, "/api/plugins").await;
            let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let plugins = value["plugins"].as_array().unwrap();
            assert_eq!(plugins.len(), 26);
            let mut names: Vec<&str> = plugins.iter().map(|p| p["name"].as_str().unwrap()).collect();
            names.sort();
            names.dedup();
            assert_eq!(names.len(), 26);
            assert!(plugins.iter().all(|p| p["module"].as_str().is_some_and(|m| !m.is_empty())));
            assert!(plugins.iter().any(|p| p["name"] == "rtmp"));
            assert!(plugins.iter().any(|p| p["name"] == "hls"));
            assert!(plugins.iter().any(|p| p["name"] == "cluster-quic"));

            let (_, body) = get(port, "/api/plugins/toggle?name=rtmp&enabled=0").await;
            assert!(String::from_utf8_lossy(&body).contains("\"code\":0"));
            let (_, body) = get(port, "/api/plugins").await;
            let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let rtmp = value["plugins"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == "rtmp")
                .unwrap();
            assert_eq!(rtmp["status"], "disabled");

            let (_, body) = get(port, "/api/plugins/toggle?name=rtmp&enabled=1").await;
            assert!(String::from_utf8_lossy(&body).contains("\"code\":0"));
        });
    unsafe { std::env::remove_var("M7S_PLUGIN_CONFIG") };
    let _ = std::fs::remove_file(cfg);
}

async fn get(port: u16, path: &str) -> (u16, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: t\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| buf[p + 4..].to_vec())
        .unwrap_or_default();
    (status, body)
}
