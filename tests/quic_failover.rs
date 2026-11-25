use std::sync::Arc;
use std::time::Duration;

use m7s_plugin_cluster::quic_ping;
use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::flv::FlvTag;
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::quic_cluster::run_on as quic_run_on;
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn quic_origin_failover_keeps_viewer() {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            // Two origins on distinct loopback addresses, same QUIC port.
            let origin_a = StreamManager::new(10);
            let origin_b = StreamManager::new(10);
            let a_stream = origin_a.get_or_create("live", "qfail");
            let b_stream = origin_b.get_or_create("live", "qfail");
            let a_tags = Arc::new(AtomicCounter::default());
            let b_tags = Arc::new(AtomicCounter::default());

            let a_task = tokio::spawn(quic_run_on(
                "127.0.0.1:9443",
                origin_a,
                "127.0.0.1:8180".to_string(),
            ));
            let b_task = tokio::spawn(quic_run_on(
                "127.0.0.2:9443",
                origin_b,
                "127.0.0.2:8180".to_string(),
            ));
            tokio::time::sleep(Duration::from_millis(400)).await;

            let a_push = tokio::spawn(push_loop(a_stream, a_tags.clone()));
            let b_push = tokio::spawn(push_loop(b_stream, b_tags.clone()));

            // Edge with both origins as seeds.
            let edge_streams = StreamManager::new(10);
            let cluster = Cluster::new(
                ClusterConfig {
                    enabled: true,
                    node_id: "edge".to_string(),
                    advertise: "127.0.0.1:8180".to_string(),
                    seeds: vec!["127.0.0.1".to_string(), "127.0.0.2".to_string()],
                    shard_redirect: false,
                    heartbeat_secs: 1,
                    relay_timeout_secs: 3,
                },
                edge_streams.clone(),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let edge_port = listener.local_addr().unwrap().port();
            let state = AppState {
                streams: edge_streams.clone(),
                cluster,
                hls: HlsManager::new(edge_streams, HlsConfig::default()),
                node_id: "edge".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            // Viewer: edge relays from origin A over QUIC.
            let mut viewer = TcpStream::connect(("127.0.0.1", edge_port)).await.unwrap();
            let req = "GET /flv/live/qfail.flv HTTP/1.1\r\nHost: edge\r\nConnection: keep-alive\r\n\r\n";
            viewer.write_all(req.as_bytes()).await.unwrap();
            let mut buf = [0u8; 8192];
            let mut header = Vec::new();
            loop {
                let n = tokio::time::timeout(Duration::from_secs(5), viewer.read(&mut buf))
                    .await
                    .expect("edge response timeout")
                    .unwrap();
                assert!(n > 0, "edge closed before headers");
                header.extend_from_slice(&buf[..n]);
                if header.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            assert!(header.starts_with(b"HTTP/1.1 200"), "{}", String::from_utf8_lossy(&header));
            let mut body = header.split_off(header.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4);
            while body.len() < 13 {
                let n = tokio::time::timeout(Duration::from_secs(3), viewer.read(&mut buf))
                    .await
                    .expect("no FLV header")
                    .unwrap();
                assert!(n > 0);
                body.extend_from_slice(&buf[..n]);
            }
            assert_eq!(&body[..3], b"FLV");

            // Health before kill is OK.
            let ping = tokio::time::timeout(
                Duration::from_secs(3),
                quic_ping("http://127.0.0.1:8180"),
            )
            .await
            .expect("origin A ping timeout")
            .expect("origin A ping");
            assert_eq!(ping["status"], "ok");

            // Kill origin A; edge must rotate to B without closing the viewer.
            a_task.abort();
            a_push.abort();
            tokio::time::sleep(Duration::from_millis(300)).await;
            let dead = tokio::time::timeout(
                Duration::from_secs(2),
                quic_ping("http://127.0.0.1:8180"),
            )
            .await;
            assert!(
                dead.is_err() || dead.unwrap().is_err(),
                "A still alive"
            );

            let before = body.len();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
            while tokio::time::Instant::now() < deadline && body.len() <= before + 64 {
                if let Ok(Ok(n)) = tokio::time::timeout(Duration::from_millis(200), viewer.read(&mut buf)).await {
                    if n == 0 {
                        panic!("edge closed viewer during failover");
                    }
                    body.extend_from_slice(&buf[..n]);
                }
            }
            assert!(body.len() > before + 64, "no media after origin failover");
            assert!(b_tags.get() > 0, "origin B never pushed");

            b_task.abort();
            b_push.abort();
        });
}

async fn push_loop(stream: Arc<m7s_rs::stream::Stream>, n: Arc<AtomicCounter>) {
    let mut ts = 0u32;
    loop {
        n.bump();
        stream.push(Arc::new(FlvTag::new(9, ts, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 4, 0x65, 1, 2, 3])));
        ts = ts.wrapping_add(40);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[derive(Default)]
struct AtomicCounter(std::sync::atomic::AtomicU64);

impl AtomicCounter {
    fn bump(&self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fn get(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}
