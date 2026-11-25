use std::sync::Arc;

use m7s_rs::flv::FlvTag;
use m7s_rs::rtsp::run;
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn rtsp_backchannel_video() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            let stream = streams.get_or_create("live", "bcv");
            stream.push(Arc::new(FlvTag::new(
                9,
                0,
                vec![0x17, 0, 0, 0, 0, 1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4, 0x68, 1, 2, 3],
            )));
            tokio::spawn(run(listener, streams.clone()));

            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            let setup = "SETUP rtsp://127.0.0.1/live/bcv/trackID=0 RTSP/1.0\r\nCSeq: 1\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n";
            s.write_all(setup.as_bytes()).await.unwrap();
            read_response(&mut s).await;
            let play = "PLAY rtsp://127.0.0.1/live/bcv RTSP/1.0\r\nCSeq: 2\r\nSession: m7s\r\n\r\n";
            s.write_all(play.as_bytes()).await.unwrap();
            let text = read_response(&mut s).await;
            assert!(text.starts_with("RTSP/1.0 200"), "{text}");

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let mut rtp = vec![0x80, 0x60, 0, 1, 0, 0, 0, 0x5a, 0, 0, 0, 7];
            rtp.extend_from_slice(&[0x65, 1, 2, 3]);
            let mut frame = vec![b'$', 0, 0, rtp.len() as u8];
            frame.extend_from_slice(&rtp);
            s.write_all(&frame).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;

            let tags = stream.cache_snapshot();
            assert!(
                tags.iter().any(|t| t.tag_type == 9 && t.data[1] == 0 && t.data.len() > 5 && t.data.len() < 19),
                "no video backchannel"
            );
        });
}

async fn read_response(s: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut b = [0u8; 1024];
    loop {
        let n = s.read(&mut b).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&b[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}
