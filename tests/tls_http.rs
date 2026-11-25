use std::sync::Arc;
use std::time::Duration;

use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run_tls};
use m7s_rs::stream::StreamManager;
use m7s_rs::tls;
use rustls::RootCertStore;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn https_health_over_self_signed_tls() {
    runtime().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let streams = StreamManager::new(10);
        let (acceptor, cert, _key) = tls::self_signed().unwrap();
        let state = AppState {
            streams: streams.clone(),
            cluster: Cluster::new(
                ClusterConfig {
                    enabled: false,
                    ..ClusterConfig::default()
                },
                streams,
            ),
            hls: HlsManager::new(StreamManager::new(10), HlsConfig::default()),
            node_id: "tls-test".to_string(),
            api_token: String::new(),
        };
        tokio::spawn(run_tls(listener, state, acceptor));

        let mut roots = RootCertStore::empty();
        roots.add(cert).unwrap();
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(config));
        let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let server_name = "localhost".try_into().unwrap();
        let mut tls =
            tokio::time::timeout(Duration::from_secs(5), connector.connect(server_name, tcp))
                .await
                .unwrap()
                .unwrap();
        tls.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        tls.read_to_end(&mut response).await.unwrap();
        let text = String::from_utf8_lossy(&response);
        assert!(text.starts_with("HTTP/1.1 200 OK"), "{text}");
        assert!(text.contains("\"status\":\"ok\""), "{text}");
    });
}
