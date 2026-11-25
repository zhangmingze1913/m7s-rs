use std::sync::Arc;

use m7s_rs::flv::FlvTag;
use m7s_rs::rtsp;
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn rtsp_describe_setup_play() {
    runtime().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let streams = StreamManager::new(10);
        let stream = streams.get_or_create("live", "test");
        let config = vec![
            0x01, 0x42, 0x00, 0x1e, 0xff, 0xe1, 0x00, 0x04, 0x67, 0x42, 0x00, 0x1e, 0x01, 0x00,
            0x03, 0x68, 0xce, 0x06, 0xe2,
        ];
        let mut seq = vec![0x17, 0, 0, 0, 0];
        seq.extend_from_slice(&config);
        stream.push(Arc::new(FlvTag::new(9, 0, seq)));
        stream.push(Arc::new(FlvTag::new(
            9,
            0,
            vec![0x17, 1, 0, 0, 0, 0, 0, 0, 4, 0x65, 1, 2, 3],
        )));
        tokio::spawn(rtsp::run(listener, streams));

        let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut response = Vec::new();
        for (method, extra) in [
            ("OPTIONS", ""),
            ("DESCRIBE", ""),
            (
                "SETUP",
                "Transport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n",
            ),
            ("PLAY", ""),
        ] {
            let req =
                format!("{method} rtsp://127.0.0.1/live/test RTSP/1.0\r\nCSeq: 1\r\n{extra}\r\n");
            socket.write_all(req.as_bytes()).await.unwrap();
            let mut buf = [0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            response.extend_from_slice(&buf[..n]);
        }
        let text = String::from_utf8_lossy(&response);
        assert!(text.contains("RTSP/1.0 200 OK"), "{text}");
        assert!(text.contains("H264/90000"), "{text}");

        if !response.contains(&b'$') {
            let mut rtp = [0u8; 32];
            let n = tokio::time::timeout(std::time::Duration::from_secs(2), socket.read(&mut rtp))
                .await
                .unwrap()
                .unwrap();
            assert!(n >= 4, "no interleaved RTP");
            assert_eq!(rtp[0], b'$');
        }
    });
}

#[test]
fn rtsp_udp_play() {
    runtime().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let streams = StreamManager::new(10);
        let stream = streams.get_or_create("live", "udp");
        let config = vec![0x01, 0x42, 0x00, 0x1e, 0xff, 0xe1, 0x00, 0x04, 0x67, 0x42, 0x00, 0x1e, 0x01, 0x00, 0x03, 0x68, 0xce, 0x06, 0xe2];
        let mut seq = vec![0x17, 0, 0, 0, 0];
        seq.extend_from_slice(&config);
        stream.push(Arc::new(FlvTag::new(9, 0, seq)));
        stream.push(Arc::new(FlvTag::new(9, 0, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 4, 0x65, 1, 2, 3])));
        tokio::spawn(rtsp::run(listener, streams));

        let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let udp_port = udp.local_addr().unwrap().port();
        for (method, extra) in [
            ("OPTIONS", ""),
            ("DESCRIBE", ""),
            ("SETUP", Box::leak(format!("Transport: RTP/AVP;unicast;client_port={udp_port}-{}\r\n", udp_port + 1).into_boxed_str())),
            ("PLAY", ""),
        ] {
            let req = format!("{method} rtsp://127.0.0.1/live/udp RTSP/1.0\r\nCSeq: 1\r\n{extra}\r\n");
            socket.write_all(req.as_bytes()).await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf).await.unwrap();
        }
        let mut packet = [0u8; 2048];
        let got = tokio::time::timeout(std::time::Duration::from_secs(2), udp.recv_from(&mut packet)).await;
        assert!(got.is_ok(), "no UDP RTP packet");
        let (n, _) = got.unwrap().unwrap();
        assert!(n >= 12);
        assert_eq!(packet[0] >> 6, 2, "not RTP v2");
    });
}

#[test]
fn rtsp_udp_audio() {
    runtime().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let streams = StreamManager::new(10);
        let stream = streams.get_or_create("live", "aud");
        stream.push(Arc::new(FlvTag::new(8, 0, vec![0xAF, 0x00, 0x12, 0x10])));
        stream.push(Arc::new(FlvTag::new(8, 0, vec![0xAF, 0x01, 1, 2, 3, 4])));
        tokio::spawn(rtsp::run(listener, streams));

        let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let udp_port = udp.local_addr().unwrap().port();
        for (method, extra, url) in [
            ("OPTIONS", "", "rtsp://127.0.0.1/live/aud"),
            ("DESCRIBE", "", "rtsp://127.0.0.1/live/aud"),
            ("SETUP", Box::leak(format!("Transport: RTP/AVP;unicast;client_port={udp_port}-{}\r\n", udp_port + 1).into_boxed_str()), "rtsp://127.0.0.1/live/aud/trackID=1"),
            ("PLAY", "", "rtsp://127.0.0.1/live/aud"),
        ] {
            let req = format!("{method} {url} RTSP/1.0\r\nCSeq: 1\r\n{extra}\r\n");
            socket.write_all(req.as_bytes()).await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf).await.unwrap();
        }
        let mut packet = [0u8; 2048];
        let got = tokio::time::timeout(std::time::Duration::from_secs(2), udp.recv_from(&mut packet)).await;
        assert!(got.is_ok(), "no UDP audio RTP");
        let (n, _) = got.unwrap().unwrap();
        assert!(n >= 12);
        assert_eq!(packet[1] & 0x7f, 97, "unexpected payload type");
    });
}

#[test]
fn rtsp_tcp_audio() {
    runtime().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let streams = StreamManager::new(10);
        let stream = streams.get_or_create("live", "atcp2");
        stream.push(Arc::new(FlvTag::new(8, 0, vec![0xAF, 0x00, 0x12, 0x10])));
        stream.push(Arc::new(FlvTag::new(8, 0, vec![0xAF, 0x01, 1, 2, 3, 4])));
        tokio::spawn(rtsp::run(listener, streams));
        let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut acc = Vec::new();
        for (method, url) in [("OPTIONS","rtsp://127.0.0.1/live/atcp2"),("DESCRIBE","rtsp://127.0.0.1/live/atcp2"),("SETUP","rtsp://127.0.0.1/live/atcp2/trackID=1"),("PLAY","rtsp://127.0.0.1/live/atcp2")] {
            let req = format!("{method} {url} RTSP/1.0\r\nCSeq: 1\r\n\r\n");
            socket.write_all(req.as_bytes()).await.unwrap();
            let mut buf=[0u8;4096];
            let n=socket.read(&mut buf).await.unwrap();
            acc.extend_from_slice(&buf[..n]);
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline && !acc.contains(&b'$') {
            let mut buf = [0u8; 2048];
            if let Ok(Ok(n)) = tokio::time::timeout(std::time::Duration::from_millis(200), socket.read(&mut buf)).await {
                acc.extend_from_slice(&buf[..n]);
            }
        }
        let pos = acc.iter().position(|b| *b == b'$').expect("no interleaved audio");
        assert!(acc.len() > pos + 14);
        assert_eq!(acc[pos + 1], 2, "audio channel");
        assert_eq!(acc[pos + 5] & 0x7f, 97, "audio payload type");
    });
}
