use std::sync::Arc;
use std::time::Duration;

use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::flv::FlvTag;
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn edge_pulls_flv_from_origin() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug"))
        .try_init();
    runtime().block_on(async {
        let origin_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_port = origin_listener.local_addr().unwrap().port();
        let edge_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let edge_port = edge_listener.local_addr().unwrap().port();

        let origin_streams = StreamManager::new(10);
        origin_streams
            .get_or_create("live", "test")
            .push(Arc::new(FlvTag::new(9, 0, vec![0x17, 1, 2, 3])));
        let origin_state = AppState {
            cluster: Cluster::new(
                ClusterConfig {
                    enabled: true,
                    node_id: "origin-1".to_string(),
                    advertise: format!("127.0.0.1:{origin_port}"),
                    seeds: Vec::new(),
                    shard_redirect: false,
                    heartbeat_secs: 5,
                    relay_timeout_secs: 5,
                },
                origin_streams.clone(),
            ),
            hls: HlsManager::new(origin_streams.clone(), HlsConfig::default()),
            streams: origin_streams,
            node_id: "origin-1".to_string(),
            api_token: String::new(),
        };
        tokio::spawn(http_run(origin_listener, origin_state));

        let edge_streams = StreamManager::new(10);
        let edge_state = AppState {
            cluster: Cluster::new(
                ClusterConfig {
                    enabled: true,
                    node_id: "edge-1".to_string(),
                    advertise: format!("127.0.0.1:{edge_port}"),
                    seeds: vec![format!("127.0.0.1:{origin_port}")],
                    shard_redirect: false,
                    heartbeat_secs: 5,
                    relay_timeout_secs: 5,
                },
                edge_streams.clone(),
            ),
            hls: HlsManager::new(edge_streams.clone(), HlsConfig::default()),
            streams: edge_streams,
            node_id: "edge-1".to_string(),
            api_token: String::new(),
        };
        tokio::spawn(http_run(edge_listener, edge_state));

        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut client = TcpStream::connect(("127.0.0.1", edge_port)).await.unwrap();
        client
            .write_all(b"GET /flv/live/test.flv HTTP/1.1\r\nHost: edge\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 256];
        let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
        loop {
            if let Some(pos) = buf.windows(3).position(|w| w == b"FLV") {
                if buf.len() > pos + 13 + 11 {
                    break;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            let n = match tokio::time::timeout(Duration::from_millis(500), client.read(&mut chunk)).await {
                Ok(Ok(n)) => n,
                _ => continue,
            };
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let header_pos = buf
            .windows(3)
            .position(|w| w == b"FLV")
            .expect("FLV header");
        assert_eq!(&buf[header_pos..header_pos + 3], b"FLV");
        assert!(
            buf.len() > header_pos + 13 + 11,
            "no tag after FLV header: {}",
            buf.len()
        );
    });
}
