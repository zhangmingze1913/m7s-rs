use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn tenant_token_isolates_stream_namespace() {
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
                        node_id: "tenant".into(),
                        advertise: format!("127.0.0.1:{port}"),
                        seeds: Vec::new(),
                        shard_redirect: false,
                        heartbeat_secs: 5,
                        relay_timeout_secs: 5,
                    },
                    streams.clone(),
                ),
                hls: HlsManager::new(streams, HlsConfig::default()),
                node_id: "tenant".into(),
                api_token: "admin".into(),
            };
            tokio::spawn(http_run(listener, state));

            let create = get(port, "/api/tenants/create?name=t1&token=tenant-token&max_viewers=1", None).await;
            assert!(create.contains("\"created\":true"), "{create}");
            let list = get(port, "/api/tenants", None).await;
            assert!(list.contains("t1"), "{list}");

            let denied = get(port, "/flv/other/x.flv", Some("tenant-token")).await;
            assert!(denied.contains("403") && denied.contains("tenant stream isolation"), "{denied}");

            let allowed = get(port, "/flv/t1/x.flv", Some("tenant-token")).await;
            assert!(allowed.contains("404"), "{allowed}");

            let deleted = get(port, "/api/tenants/delete?name=t1", None).await;
            assert!(deleted.contains("\"removed\":true"), "{deleted}");
        });
}

async fn get(port: u16, path: &str, tenant: Option<&str>) -> String {
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let auth = match tenant {
        Some(t) => format!("X-API-Token: {t}\r\n"),
        None => "X-API-Token: admin\r\n".to_string(),
    };
    socket
        .write_all(format!("GET {path} HTTP/1.1\r\nHost: t\r\n{auth}Connection: close\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut buf = Vec::new();
    socket.read_to_end(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf).to_string()
}
