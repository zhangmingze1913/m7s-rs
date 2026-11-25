use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn room_create_list_close() {
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
                node_id: "room".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            let (_, body) = get(port, "/api/live/room/create?room=it-room&title=Demo").await;
            assert!(String::from_utf8_lossy(&body).contains("\"created\":true"));

            let (_, body) = get(port, "/api/live/rooms").await;
            let text = String::from_utf8_lossy(&body);
            assert!(text.contains("it-room"), "{text}");
            assert!(text.contains("Demo"), "{text}");

            let (_, body) = get(port, "/api/live/room/close?room=it-room").await;
            assert!(String::from_utf8_lossy(&body).contains("\"closed\":true"));
            let (_, body) = get(port, "/api/live/rooms").await;
            assert!(String::from_utf8_lossy(&body).contains("\"closed\":true"));
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

#[test]
fn live_seats_mute_kick() {
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
                node_id: "seat".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            let (_, body) = get(port, "/api/live/join?room=seat-room&user=alice").await;
            assert!(String::from_utf8_lossy(&body).contains("\"joined\":true"));
            let (_, body) = get(port, "/api/live/seat/take?room=seat-room&user=alice&index=1").await;
            let text = String::from_utf8_lossy(&body);
            assert!(text.contains("\"ok\":true"), "{text}");
            assert!(text.contains("alice"), "{text}");

            let (_, body) = get(port, "/api/live/mute?room=seat-room&user=alice&on=1").await;
            assert!(String::from_utf8_lossy(&body).contains("\"muted\":true"));
            let (_, body) = get(port, "/api/live/robots?room=seat-room&count=5").await;
            assert!(String::from_utf8_lossy(&body).contains("\"robots\":5"));
            let (_, body) = get(port, "/api/live/status?room=seat-room").await;
            let text = String::from_utf8_lossy(&body);
            assert!(text.contains("alice"), "{text}");
            assert!(text.contains("\"robots\":5"), "{text}");

            let (_, body) = get(port, "/api/live/kick?room=seat-room&user=alice").await;
            assert!(String::from_utf8_lossy(&body).contains("\"members\":0"));
            let (_, body) = get(port, "/api/live/status?room=seat-room").await;
            assert!(!String::from_utf8_lossy(&body).contains("alice"));
        });
}

#[test]
fn live_chat_moderation_http() {
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
                node_id: "chat".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            let (_, body) = get(port, "/api/live/chat/send?room=chat-room&user=alice&text=hello").await;
            assert!(String::from_utf8_lossy(&body).contains("\"text\":\"hello\""));
            let (_, body) = get(port, "/api/live/chat/list?room=chat-room").await;
            assert!(String::from_utf8_lossy(&body).contains("hello"));
            let (_, body) = get(port, "/api/live/chat/send?room=chat-room&user=alice&text=badword").await;
            assert!(String::from_utf8_lossy(&body).contains("\"code\":403"));
            let (_, _) = get(port, "/api/live/mute?room=chat-room&user=alice&on=1").await;
            let (_, body) = get(port, "/api/live/chat/send?room=chat-room&user=alice&text=again").await;
            assert!(String::from_utf8_lossy(&body).contains("muted"));
        });
}

#[test]
fn mix_http_generates_mp4() {
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
                node_id: "mix".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));
            let (status, body) = get(port, "/api/mix?input=lavfi:color=red:s=64x64:d=1,lavfi:color=blue:s=64x64:d=1&secs=1").await;
            let text = String::from_utf8_lossy(&body);
            assert_eq!(status, 200, "{text}");
            assert!(text.contains("\"code\":0"), "{text}");
            assert!(text.contains("data/mix"), "{text}");
        });
}

#[test]
fn room_auto_mix_uses_room_streams() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            m7s_plugin_meeting::set_stream("mix-room-a", "lavfi:color=red:s=64x64:d=1");
            m7s_plugin_meeting::set_stream("mix-room-b", "lavfi:color=blue:s=64x64:d=1");
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            let state = AppState {
                streams: streams.clone(),
                cluster: Cluster::new(ClusterConfig { enabled: false, ..ClusterConfig::default() }, StreamManager::new(10)),
                hls: HlsManager::new(streams, HlsConfig::default()),
                node_id: "room-mix".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));
            let (status, body) = get(port, "/api/live/mix?room=mix-room-a&other=mix-room-b&secs=1").await;
            let text = String::from_utf8_lossy(&body);
            assert_eq!(status, 200, "{text}");
            assert!(text.contains("\"code\":0"), "{text}");
            assert!(text.contains("data/mix") && text.contains("room-"), "{text}");
        });
}

#[test]
fn meeting_moderation_http() {
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
                node_id: "meet".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));
            let (_, _) = get(port, "/api/meeting/join?room=mod-room&user=a").await;
            let (_, body) = get(port, "/api/meeting/mute?room=mod-room&user=a&on=1").await;
            assert!(String::from_utf8_lossy(&body).contains("\"muted\":true"));
            let (_, body) = get(port, "/api/meeting/hand?room=mod-room&user=a&on=1").await;
            assert!(String::from_utf8_lossy(&body).contains("\"hands\":1"));
            let (_, body) = get(port, "/api/meeting/status?room=mod-room").await;
            assert!(String::from_utf8_lossy(&body).contains("\"muted\":1"));
            let (_, _) = get(port, "/api/meeting/kick?room=mod-room&user=a").await;
            let (_, body) = get(port, "/api/meeting/status?room=mod-room").await;
            assert!(String::from_utf8_lossy(&body).contains("\"members\":0"));
        });
}

#[test]
fn gift_ranking_http() {
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
                node_id: "gift".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));
            let _ = get(port, "/api/live/gift?room=rank-room&user=a&gift=10&count=1").await;
            let _ = get(port, "/api/live/gift?room=rank-room&user=b&gift=1&count=5").await;
            let (_, body) = get(port, "/api/live/gift/ranking?room=rank-room").await;
            let text = String::from_utf8_lossy(&body);
            let b = text.find("\"user\":\"b\"").unwrap();
            let a = text.find("\"user\":\"a\"").unwrap();
            assert!(b < a, "{text}");
        });
}
