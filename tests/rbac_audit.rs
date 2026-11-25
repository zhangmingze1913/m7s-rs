use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn rbac_and_audit() {
    let dir = std::env::temp_dir().join(format!("m7s-rbac-{}", std::process::id()));
    let user_db = dir.join("users.json");
    let audit = dir.join("audit.log");
    unsafe {
        std::env::set_var("M7S_JWT_SECRET", "test-secret");
        std::env::set_var("M7S_USER_DB", &user_db);
        std::env::set_var("M7S_AUDIT_LOG", &audit);
    }
    m7s_rs::users::add_with_role("viewer", "pw", "viewer").unwrap();

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
                node_id: "rbac".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));
            let (status, body) = get(port, "/api/login?username=viewer&password=pw", None).await;
            assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
            let viewer = token_json(&String::from_utf8_lossy(&body));
            assert!(!viewer.is_empty());

            let admin = m7s_rs::auth::issue("test-secret", "admin", 3600);
            assert_eq!(get(port, "/api/admin/overview", Some(&viewer)).await.0, 403);
            assert_eq!(get(port, "/api/admin/overview", Some(&admin)).await.0, 200);
            assert_eq!(get(port, "/api/audit", Some(&viewer)).await.0, 403);

            assert_eq!(
                get(port, "/api/stream/delete?stream=live/missing", Some(&admin)).await.0,
                200
            );
            let (status, body) = get(port, "/api/audit?limit=10", Some(&admin)).await;
            assert_eq!(status, 200);
            let text = String::from_utf8_lossy(&body);
            assert!(text.contains("stream.delete"), "{text}");
            let (status, body) = get(port, "/api/audit/export", Some(&admin)).await;
            assert_eq!(status, 200);
            let csv = String::from_utf8_lossy(&body);
            assert!(csv.starts_with("ts,actor,role,action,target,ok"), "{csv}");
            assert!(csv.contains("stream.delete"), "{csv}");
        });
}

fn token_json(text: &str) -> String {
    let start = text.find("\"token\":\"").map(|i| i + 9).unwrap_or(0);
    let rest = &text[start..];
    rest.split('"').next().unwrap_or("").to_string()
}

async fn get(port: u16, path: &str, token: Option<&str>) -> (u16, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: t\r\n{auth}\r\n").as_bytes())
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
