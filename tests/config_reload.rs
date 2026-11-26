use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn config_reload_applies_hls_settings() {
    let path = std::env::temp_dir().join(format!("m7s-reload-{}.toml", std::process::id()));
    write_config(&path, 7);
    m7s_rs::config::set_reload_path(path.clone());

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
                cluster: Cluster::new(
                    ClusterConfig {
                        enabled: false,
                        node_id: "reload".into(),
                        advertise: format!("127.0.0.1:{port}"),
                        seeds: Vec::new(),
                        shard_redirect: false,
                        heartbeat_secs: 5,
                        relay_timeout_secs: 5,
                    },
                    streams.clone(),
                ),
                hls: HlsManager::new(streams, HlsConfig::default()),
                node_id: "reload".into(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            let body = get(port, "/api/config/reload").await;
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(v["code"], 0, "{body}");
            assert_eq!(v["hls"]["segment_secs"], 7);

            write_config(&path, 9);
            let body = get(port, "/api/config/reload").await;
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(v["hls"]["segment_secs"], 9, "{body}");
        });

    let _ = std::fs::remove_file(path);
}

fn write_config(path: &std::path::Path, segment_secs: u64) {
    std::fs::write(
        path,
        format!(
            "[server]\nhttp_bind = \"127.0.0.1:0\"\nrtmp_bind = \"127.0.0.1:0\"\n\n[hls]\nenabled = true\nsegment_secs = {segment_secs}\nmax_segments = 4\n"
        ),
    )
    .unwrap();
}

async fn get(port: u16, path: &str) -> String {
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    socket
        .write_all(format!("GET {path} HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut buf = Vec::new();
    socket.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    text.split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default()
}
