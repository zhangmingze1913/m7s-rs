use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn api_token_enforced() {
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
                    ClusterConfig { enabled: false, ..ClusterConfig::default() },
                    StreamManager::new(10),
                ),
                hls: HlsManager::new(streams, HlsConfig::default()),
                node_id: "auth".to_string(),
                api_token: "secret".to_string(),
            };
            tokio::spawn(http_run(listener, state));

            assert_eq!(get(port, "/health", None).await.0, 200);
            assert_eq!(get(port, "/api/streams", None).await.0, 401);
            assert_eq!(
                get(port, "/api/streams", Some("Bearer secret")).await.0,
                200
            );
            assert_eq!(
                get(port, "/api/streams", Some("X-API-Token: secret")).await.0,
                200
            );
            assert_eq!(
                get(port, "/api/streams", Some("Bearer wrong")).await.0,
                401
            );
        });
}

async fn get(port: u16, path: &str, auth: Option<&str>) -> (u16, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let auth_line = match auth {
        Some(v) if v.starts_with("Bearer") => format!("Authorization: {v}\r\n"),
        Some(v) => format!("{v}\r\n"),
        None => String::new(),
    };
    s.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: t\r\n{auth_line}\r\n").as_bytes(),
    )
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
