
use m7s_rs::rtsp_pull::pull_with_retry;
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn rtsp_pull_reconnects() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(mock_rtsp(listener, 2));
            let streams = StreamManager::new(20);
            let pull_streams = streams.clone();
            tokio::spawn(async move {
                let _ = pull_with_retry(
                    &format!("rtsp://127.0.0.1:{port}/live/mock"),
                    pull_streams,
                    "live",
                    "pulled",
                    2,
                )
                .await;
            });
            tokio::time::sleep(std::time::Duration::from_millis(1800)).await;
            let dest = streams.get("live", "pulled").expect("pulled");
            let tags = dest.cache_snapshot();
            let frames = tags
                .iter()
                .filter(|t| t.tag_type == 9 && t.data.len() > 5 && t.data.len() < 19 && t.data[1] == 0)
                .count();
            assert!(frames >= 2, "frames={frames} tags={}", tags.len());
        });
}

async fn mock_rtsp(listener: TcpListener, connections: usize) {
    for n in 0..connections {
        let (mut socket, _) = listener.accept().await.unwrap();
        loop {
            let Some(head) = read_head(&mut socket).await else { return };
            let method = head.split_whitespace().next().unwrap_or("");
            match method {
                "DESCRIBE" => {
                    let sdp = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=mock\r\nt=0 0\r\na=control:*\r\nm=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\na=fmtp:96 packetization-mode=1;sprop-parameter-sets=Z0IAHg==,aAECAw==\r\na=control:trackID=0\r\n";
                    socket
                        .write_all(format!("RTSP/1.0 200 OK\r\nCSeq: 1\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{sdp}", sdp.len()).as_bytes())
                        .await
                        .unwrap();
                }
                "SETUP" => {
                    socket
                        .write_all(b"RTSP/1.0 200 OK\r\nCSeq: 2\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\nSession: m\r\n\r\n")
                        .await
                        .unwrap();
                }
                "PLAY" => {
                    socket
                        .write_all(b"RTSP/1.0 200 OK\r\nCSeq: 3\r\nSession: m\r\n\r\n")
                        .await
                        .unwrap();
                    let mut rtp = vec![0x80, 0x60, 0, 1, 0, 0, 0, (n as u32 * 90).to_be_bytes()[3], 0, 0, 0, 1];
                    rtp.extend_from_slice(&[0x65, 1, 2]);
                    let mut frame = vec![b'$', 0, 0, rtp.len() as u8];
                    frame.extend_from_slice(&rtp);
                    socket.write_all(&frame).await.unwrap();
                    socket.shutdown().await.ok();
                    break;
                }
                _ => break,
            }
        }
    }
}

async fn read_head(socket: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut b = [0u8; 1024];
    loop {
        let n = socket.read(&mut b).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&b[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return Some(String::from_utf8_lossy(&buf).to_string());
        }
    }
}
