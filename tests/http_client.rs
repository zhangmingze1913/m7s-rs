use std::io::{Read, Write};

use m7s_rs::http_client::HttpClientImpl;
use m7s_sdk::HttpClient;

#[test]
fn simple_http_client_get() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = s.read(&mut buf);
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nhello-http")
            .unwrap();
    });
    let body = HttpClientImpl.get(&format!("http://127.0.0.1:{port}/probe")).unwrap();
    assert_eq!(body, b"hello-http");
    server.join().unwrap();

    let ctx = m7s_sdk::EngineContext::new("n", "http://x", "rtsp://x")
        .with_http(std::sync::Arc::new(HttpClientImpl));
    assert!(ctx.http().is_some());
}
