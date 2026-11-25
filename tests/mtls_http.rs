use std::sync::Arc;
use std::time::Duration;

use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run_tls};
use m7s_rs::stream::StreamManager;
use m7s_rs::tls;
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
fn mtls_accepts_client_cert_and_rejects_missing_cert() {
    runtime().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let acceptor = tls::load_server_with_client_auth(
            std::path::Path::new("tests/fixtures/server.pem"),
            std::path::Path::new("tests/fixtures/server-key.pem"),
            std::path::Path::new("tests/fixtures/ca.pem"),
        )
        .unwrap();
        let streams = StreamManager::new(10);
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
            node_id: "mtls".to_string(),
            api_token: String::new(),
        };
        tokio::spawn(run_tls(listener, state, acceptor));

        let client = tls::load_client(
            std::path::Path::new("tests/fixtures/client.pem"),
            std::path::Path::new("tests/fixtures/client-key.pem"),
            std::path::Path::new("tests/fixtures/ca.pem"),
        )
        .unwrap();
        let connector = TlsConnector::from(Arc::new(client));
        let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut tls = connector
            .connect("localhost".try_into().unwrap(), tcp)
            .await
            .unwrap();
        tls.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        tls.read_to_end(&mut response).await.unwrap();
        let text = String::from_utf8_lossy(&response);
        assert!(text.starts_with("HTTP/1.1 200 OK"), "{text}");

        let no_cert = rustls::ClientConfig::builder()
            .with_root_certificates({
                let mut roots = rustls::RootCertStore::empty();
                let pem = std::fs::read_to_string("tests/fixtures/ca.pem").unwrap();
                let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
                let der = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body)
                    .unwrap();
                roots
                    .add(rustls::pki_types::CertificateDer::from(der))
                    .unwrap();
                roots
            })
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(no_cert));
        let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            connector.connect("localhost".try_into().unwrap(), tcp),
        )
        .await
        .unwrap();
        if let Ok(mut tls) = result {
            let _ = tls
                .write_all(b"GET /health HTTP/1.1\\r\\nHost: localhost\\r\\n\\r\\n")
                .await;
            let mut body = Vec::new();
            let read =
                tokio::time::timeout(Duration::from_secs(2), tls.read_to_end(&mut body)).await;
            assert!(
                read.is_err() || read.unwrap().is_err() || body.is_empty(),
                "mTLS must reject missing client cert"
            );
        }
    });
}
