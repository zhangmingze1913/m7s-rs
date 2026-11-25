use std::sync::Arc;

use bytes::Bytes;
use h2::client;
use http::Request;
use m7s_rs::grpc::run_tls as grpc_run_tls;
use m7s_rs::stream::StreamManager;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;

#[test]
fn grpc_tls_roundtrip() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let streams = StreamManager::new(10);
            streams.get_or_create("live", "grpc-tls");

            let mut chain_reader =
                std::io::BufReader::new(std::fs::File::open("tests/fixtures/server.pem").unwrap());
            let chain: Vec<_> = rustls_pemfile::certs(&mut chain_reader)
                .collect::<Result<_, _>>()
                .unwrap();
            let mut key_reader =
                std::io::BufReader::new(std::fs::File::open("tests/fixtures/server-key.pem").unwrap());
            let key = rustls_pemfile::private_key(&mut key_reader).unwrap().unwrap();
            let server = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(chain, key)
                .unwrap();
            let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
            tokio::spawn(grpc_run_tls(listener, streams, acceptor));

            let mut ca_reader =
                std::io::BufReader::new(std::fs::File::open("tests/fixtures/ca.pem").unwrap());
            let mut roots = rustls::RootCertStore::empty();
            for cert in rustls_pemfile::certs(&mut ca_reader) {
                roots.add(cert.unwrap()).unwrap();
            }
            let client_cfg = rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            let connector = TlsConnector::from(Arc::new(client_cfg));
            let tcp = TcpStream::connect(addr).await.unwrap();
            let server_name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
            let tls = connector.connect(server_name, tcp).await.unwrap();
            let (mut send_request, conn) = client::handshake(tls).await.unwrap();
            tokio::spawn(async move { let _ = conn.await; });

            let req = Request::builder()
                .method("POST")
                .uri("/m7s.StreamService/ListStreams")
                .header("content-type", "application/grpc")
                .header("te", "trailers")
                .body(())
                .unwrap();
            let (response, mut body) = send_request.send_request(req, false).unwrap();
            body.send_data(Bytes::from_static(&[0, 0, 0, 0, 0]), true).unwrap();
            let response = response.await.unwrap();
            let mut resp_body = response.into_body();
            let mut data = Vec::new();
            while let Some(chunk) = resp_body.data().await {
                data.extend_from_slice(&chunk.unwrap());
            }
            assert!(data.len() > 5);
            let trailers = resp_body.trailers().await.unwrap().unwrap();
            assert_eq!(trailers.get("grpc-status").unwrap(), "0");
        });
}
