use std::time::{Duration, Instant};
use m7s_rs::srt;
use m7s_rs::stream::StreamManager;
use sapflow_srt::{Caller, CallerConfig};

#[test]
fn srt_publish_ts() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).try_init();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let probe = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let streams = StreamManager::new(10);
        let stream = streams.get_or_create("live", "srt");
        let addr = format!("127.0.0.1:{port}");
        tokio::spawn(async move { let _ = srt::run(&addr, streams).await; });
        tokio::time::sleep(Duration::from_millis(100)).await;

        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let cfg = CallerConfig { stream_id: "live/srt".to_string(), ..Default::default() };
        let mut caller = Caller::new(cfg, peer, Instant::now()).unwrap();
        let mut buf = [0u8; 2048];
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !caller.is_connected() {
            while let Some((to, data)) = caller.poll_transmit(Instant::now()) { let _ = socket.send_to(&data, to).await; }
            if let Ok(Ok((n, from))) = tokio::time::timeout(Duration::from_millis(20), socket.recv_from(&mut buf)).await { caller.handle_datagram(Instant::now(), from, &buf[..n]); }
            caller.handle_timeout(Instant::now());
            while caller.poll_event().is_some() {}
        }
        assert!(caller.is_connected(), "handshake");

        let mut mux = m7s_codec::ts::TsMuxer::new();
        let mut ts = Vec::new();
        mux.write_tables(&mut ts);
        for n in 0..10u64 {
            let mut annexb = vec![0,0,0,1,0x67,0x42,0,0x1e,0xE1,0x01,0x40,0,0,0,1,0x68,0xCE,0x3C,0x80,0,0,0,1,0x65,0x88,0x84,0x00];
            annexb.push(n as u8);
            mux.write_video(&mut ts, &annexb, n * 3600, n * 3600);
        }
        let mut it = ts.chunks(1024).map(|c| c.to_vec());
        let mut pending = it.next();
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            while let Some((to, data)) = caller.poll_transmit(Instant::now()) { let _ = socket.send_to(&data, to).await; }
            if caller.stats().send.buffered_packets < 8 {
                if let Some(c) = pending.take() { caller.enqueue(Instant::now(), c).unwrap(); }
                pending = it.next();
            }
            if let Ok(Ok((n, from))) = tokio::time::timeout(Duration::from_millis(10), socket.recv_from(&mut buf)).await { caller.handle_datagram(Instant::now(), from, &buf[..n]); }
            caller.handle_timeout(Instant::now());
            while caller.poll_event().is_some() {}
            if stream.cache_snapshot().iter().any(|t| t.tag_type == 9) { eprintln!("GOT_TAG"); return; }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        panic!("no tag");
    });
}
