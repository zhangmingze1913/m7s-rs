
use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn serves_web_sdk() {
    let gift_file = std::path::PathBuf::from(format!("{}/../../target/web-sdk-gifts-{}.json", env!("CARGO_MANIFEST_DIR"), std::process::id()));
    let _ = std::fs::remove_file(&gift_file);
    unsafe {
        std::env::set_var("M7S_GIFT_FILE", &gift_file);
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            streams.get_or_create("live", "gone");
            let state = AppState {
                streams: streams.clone(),
                cluster: Cluster::new(
                    ClusterConfig {
                        enabled: false,
                        ..ClusterConfig::default()
                    },
                    StreamManager::new(10),
                ),
                hls: HlsManager::new(streams, HlsConfig::default()),
                node_id: "test".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            let (status, js) = get(port, "/sdk.js").await;
            assert_eq!(status, 200);
            let js = String::from_utf8_lossy(&js);
            assert!(js.contains("M7S"), "{js}");
            assert!(js.contains("whep"), "{js}");

            let (status, html) = get(port, "/player").await;
            assert_eq!(status, 200);
            assert!(String::from_utf8_lossy(&html).contains("m7s Web SDK"));

            let (status, caps) = get(port, "/api/webrtc/capabilities").await;
            assert_eq!(status, 200);
            let caps = String::from_utf8_lossy(&caps);
            assert!(caps.contains("\"simulcast_rid\":true"), "{caps}");
            assert!(caps.contains("\"svc\":false"), "{caps}");
            assert!(caps.contains("\"twcc_per_packet\":true"), "{caps}");

            let (_, j) = get(port, "/api/live/join?room=r&user=u").await;
            assert!(String::from_utf8_lossy(&j).contains("\"joined\":true"));
            let (_, g) = get(port, "/api/live/gift?room=r&user=u&gift=1&count=2").await;
            assert!(String::from_utf8_lossy(&g).contains("\"total\":2"));
            let (_, s) = get(port, "/api/live/status?room=r").await;
            assert!(String::from_utf8_lossy(&s).contains("\"members\":1"));

            let (_, m) = get(port, "/api/meeting/join?room=m&user=a").await;
            assert!(String::from_utf8_lossy(&m).contains("\"members\":1"));
            let (_, m) = get(port, "/api/meeting/publish?room=m&user=a&on=1").await;
            assert!(String::from_utf8_lossy(&m).contains("\"publishers\":1"));

            let (_, rooms) = get(port, "/api/live/rooms").await;
            assert!(String::from_utf8_lossy(&rooms).contains("\"r\""));
            let (_, mrooms) = get(port, "/api/meeting/rooms").await;
            assert!(String::from_utf8_lossy(&mrooms).contains("\"m\""));

            let (_, ms) = get(port, "/api/meeting/stream?room=m&stream=live/cam").await;
            assert!(String::from_utf8_lossy(&ms).contains("live/cam"));
            let (_, ms) = get(port, "/api/meeting/stream?room=m").await;
            assert!(String::from_utf8_lossy(&ms).contains("live/cam"));

            let (status, admin) = get(port, "/admin").await;
            assert_eq!(status, 200);
            let admin = String::from_utf8_lossy(&admin);
            assert!(admin.contains("Plugins"), "{admin}");
            assert!(admin.contains("Authorization"), "{admin}");

            let (status, body) = get(port, "/api/stream/delete?stream=live/gone").await;
            assert_eq!(status, 200);
            let body = String::from_utf8_lossy(&body);
            assert!(body.contains("\"removed\":true"), "{body}");
            let (_, list) = get(port, "/api/streams").await;
            assert!(String::from_utf8_lossy(&list).contains("\"streams\":[]"));
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
