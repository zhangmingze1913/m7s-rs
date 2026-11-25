use std::sync::Arc;

use m7s_rs::flv::FlvTag;
use m7s_rs::rtsp::run;
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

#[test]
fn rtsp_multicast_video() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let probe = UdpSocket::bind("0.0.0.0:0").await.unwrap();
            let _ = probe.connect("8.8.8.8:80").await;
            let iface = match probe.local_addr().map(|a| a.ip()) {
                Ok(std::net::IpAddr::V4(v)) => v,
                _ => std::net::Ipv4Addr::LOCALHOST,
            };
            unsafe { std::env::set_var("M7S_RTSP_MULTICAST_IF", iface.to_string()) };
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            let stream = streams.get_or_create("live", "mc");
            stream.push(Arc::new(FlvTag::new(
                9,
                0,
                vec![0x17, 0, 0, 0, 0, 1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4, 0x68, 1, 2, 3],
            )));
            stream.push(Arc::new(FlvTag::new(9, 40, vec![0, 0, 0, 2, 0x65, 0x88])));
            tokio::spawn(run(listener, streams));

            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            let setup = "SETUP rtsp://127.0.0.1/live/mc/trackID=0 RTSP/1.0\r\nCSeq: 1\r\nTransport: RTP/AVP;multicast\r\n\r\n";
            s.write_all(setup.as_bytes()).await.unwrap();
            let text = read_response(&mut s).await;
            assert!(text.contains("multicast"), "{text}");
            let group: std::net::Ipv4Addr = text
                .split("destination=")
                .nth(1)
                .and_then(|s| s.split(';').next())
                .and_then(|s| s.parse().ok())
                .expect("destination");
            let rtp_port: u16 = text
                .split("port=")
                .nth(1)
                .and_then(|s| s.split('-').next())
                .and_then(|s| s.parse().ok())
                .expect("port");

            let play = "PLAY rtsp://127.0.0.1/live/mc RTSP/1.0\r\nCSeq: 2\r\nSession: m7s\r\n\r\n";
            s.write_all(play.as_bytes()).await.unwrap();
            let text = read_response(&mut s).await;
            assert!(text.starts_with("RTSP/1.0 200"), "{text}");
            assert!(group.is_multicast());
            assert!(rtp_port >= 30000);
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
