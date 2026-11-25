use m7s_rs::rtsp_pull::pull_with_retry;
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn pull_sends_rtcp_receiver_report() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (tx, rx) = tokio::sync::oneshot::channel();
            tokio::spawn(mock(listener, tx));
            let streams = StreamManager::new(5);
            let pull_streams = streams.clone();
            tokio::spawn(async move {
                let _ = pull_with_retry(
                    &format!("rtsp://127.0.0.1:{port}/live/rtcp"),
                    pull_streams,
                    "live",
                    "pulled",
                    0,
                )
                .await;
            });
            let got = tokio::time::timeout(std::time::Duration::from_secs(2), rx)
                .await
                .expect("rr timeout")
                .unwrap();
            assert!(got, "no RR");
        });
}

async fn mock(listener: TcpListener, tx: tokio::sync::oneshot::Sender<bool>) {
    let (mut s, _) = listener.accept().await.unwrap();
    loop {
        let Some(head) = read_head(&mut s).await else { return };
        let method = head.split_whitespace().next().unwrap_or("");
        match method {
            "DESCRIBE" => {
                let sdp = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=mock\r\nt=0 0\r\nm=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\na=fmtp:96 packetization-mode=1;sprop-parameter-sets=Z0IAHg==,aAECAw==\r\na=control:trackID=0\r\n";
                s.write_all(format!("RTSP/1.0 200 OK\r\nCSeq: 1\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{sdp}", sdp.len()).as_bytes()).await.unwrap();
            }
            "SETUP" => {
                s.write_all(b"RTSP/1.0 200 OK\r\nCSeq: 2\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\nSession: m\r\n\r\n").await.unwrap();
            }
            "PLAY" => {
                s.write_all(b"RTSP/1.0 200 OK\r\nCSeq: 3\r\nSession: m\r\n\r\n").await.unwrap();
                let mut sr = vec![0x80, 200, 0, 6, 0, 0, 0, 9];
                sr.extend_from_slice(&[0u8; 20]);
                let mut frame = vec![b'$', 1, 0, sr.len() as u8];
                frame.extend_from_slice(&sr);
                s.write_all(&frame).await.unwrap();
                let rr = read_interleaved(&mut s).await;
                let _ = tx.send(rr);
                return;
            }
            _ => return,
        }
    }
}

async fn read_head(s: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut b = [0u8; 1024];
    loop {
        let n = s.read(&mut b).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&b[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return Some(String::from_utf8_lossy(&buf).to_string());
        }
    }
}

async fn read_interleaved(s: &mut TcpStream) -> bool {
    let mut hdr = [0u8; 4];
    if s.read_exact(&mut hdr).await.is_err() {
        return false;
    }
    if hdr[0] != b'$' {
        return false;
    }
    let len = u16::from_be_bytes([hdr[2], hdr[3]]) as usize;
    let mut payload = vec![0u8; len];
    if s.read_exact(&mut payload).await.is_err() {
        return false;
    }
    let channel = hdr[1];
    payload.len() >= 2 && payload[1] == 201 && (channel == 1 || channel == 3)
}
