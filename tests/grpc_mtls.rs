use std::sync::Arc;

use bytes::Bytes;
use h2::client;
use http::Request;
use m7s_rs::grpc::run_tls as grpc_run_tls;
use m7s_rs::stream::StreamManager;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;

#[test]
fn grpc_mtls_requires_client_cert() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let streams = StreamManager::new(5);
            streams.get_or_create("live", "mtls");
            let acceptor = m7s_rs::tls::load_server_with_client_auth(
                std::path::Path::new("tests/fixtures/server.pem"),
                std::path::Path::new("tests/fixtures/server-key.pem"),
                std::path::Path::new("tests/fixtures/ca.pem"),
            )
            .unwrap();
            tokio::spawn(grpc_run_tls(listener, streams, acceptor));

            // With client cert.
            let cfg = m7s_rs::tls::load_client(
                std::path::Path::new("tests/fixtures/client.pem"),
                std::path::Path::new("tests/fixtures/client-key.pem"),
                std::path::Path::new("tests/fixtures/ca.pem"),
            )
            .unwrap();
            let connector = TlsConnector::from(Arc::new(cfg));
            let tcp = TcpStream::connect(addr).await.unwrap();
            let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
            let tls = connector.connect(name, tcp).await.unwrap();
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
            assert_eq!(response.status(), 200);

        });
}
