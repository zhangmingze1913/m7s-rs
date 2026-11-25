use std::sync::Arc;

use m7s_rs::flv::FlvTag;
use m7s_rs::rtsp::{RtspAuth, run_with_auth};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn md5_hex(s: &str) -> String {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

#[test]
fn rtsp_digest_auth() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            let stream = streams.get_or_create("live", "auth");
            stream.push(Arc::new(FlvTag::new(
                9,
                0,
                vec![0x17, 0, 0, 0, 0, 1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4, 0x68, 1, 2, 3],
            )));
            tokio::spawn(run_with_auth(
                listener,
                streams,
                Some(RtspAuth { user: "admin".to_string(), password: "secret".to_string() }),
            ));

            let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            let options = request(&mut socket, "OPTIONS", "").await;
            assert!(options.starts_with("RTSP/1.0 200"), "{options}");

            let challenge = request(&mut socket, "DESCRIBE", "").await;
            assert!(challenge.starts_with("RTSP/1.0 401"), "{challenge}");
            let nonce = challenge
                .split("nonce=\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .expect("nonce");
            let uri = "rtsp://127.0.0.1/live/auth";
            let nc = "00000001";
            let cnonce = "abcdef";
            let ha1 = md5_hex("admin:m7s:secret");
            let ha2 = md5_hex(&format!("DESCRIBE:{uri}"));
            let response = md5_hex(&format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}"));
            let auth = format!(
                "Authorization: Digest username=\"admin\", realm=\"m7s\", nonce=\"{nonce}\", uri=\"{uri}\", response=\"{response}\", qop=auth, nc={nc}, cnonce=\"{cnonce}\"\r\n"
            );
            let ok = request(&mut socket, "DESCRIBE", &auth).await;
            assert!(ok.starts_with("RTSP/1.0 200"), "{ok}");
        });
}

async fn request(socket: &mut TcpStream, method: &str, extra: &str) -> String {
    let req = format!(
        "{method} rtsp://127.0.0.1/live/auth RTSP/1.0\r\nCSeq: 1\r\n{extra}\r\n"
    );
    socket.write_all(req.as_bytes()).await.unwrap();
    let mut buf = [0u8; 8192];
    let n = socket.read(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf[..n]).to_string()
}
