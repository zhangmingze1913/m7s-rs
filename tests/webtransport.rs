
use std::sync::Arc;

use bytes::Bytes;
use m7s_rs::stream::StreamManager;
use m7s_rs::webtransport::run as wt_run;

#[test]
fn datagram_echo() {
    let cfg = std::env::temp_dir().join(format!("m7s-wt-toggle-{}.json", std::process::id()));
    unsafe { std::env::set_var("M7S_PLUGIN_CONFIG", &cfg) };
    std::fs::write(&cfg, b"[]").unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let streams = StreamManager::new(10);
            streams.get_or_create("live", "wt");
            tokio::spawn(wt_run("127.0.0.1:4443", streams));
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;

            let mut certs = Vec::new();
            let mut reader =
                std::io::BufReader::new(std::fs::File::open("tests/fixtures/server.pem").unwrap());
            for cert in rustls_pemfile::certs(&mut reader) {
                certs.push(cert.unwrap());
            }
            let client = web_transport_quinn::ClientBuilder::new()
                .with_server_certificates(certs)
                .unwrap();
            let url = url::Url::parse("https://127.0.0.1:4443/live/wt").unwrap();
            let session = client
                .connect(url)
                .await
                .unwrap();
            let payload = Bytes::from_static(b"hello-wt");
            session.send_datagram_wait(payload.clone()).await.unwrap();
            let got = session.read_datagram().await.unwrap();
            assert_eq!(got, payload);
        });
}

#[test]
fn webtransport_egress_pushes_stream() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let probe = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            let port = probe.local_addr().unwrap().port();
            drop(probe);
            let dest = StreamManager::new(10);
            let dest_stream = dest.get_or_create("live", "wtout");
            let addr = format!("127.0.0.1:{port}");
            tokio::spawn(async move {
                let _ = wt_run(&addr, dest).await;
            });
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;

            let source = StreamManager::new(10);
            let src_stream = source.get_or_create("live", "wtsrc");
            let cfg = vec![0x17, 0, 0, 0, 0, 1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4, 0x68, 1, 2, 3];
            src_stream.push(Arc::new(m7s_rs::flv::FlvTag::new(9, 0, cfg)));
            let url = format!("https://127.0.0.1:{port}/live/wtout");
            let src = src_stream.clone();
            tokio::spawn(async move {
                let _ = m7s_rs::webtransport::push(src, &url).await;
            });
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            for ts in (0u32..4000).step_by(250) {
                let mut data = vec![0x17, 1, 0, 0, 0, 0, 0, 0, 4, if ts % 1000 == 0 { 0x65 } else { 0x41 }, 1, 2, 3];
                data[8] = 4;
                src_stream.push(Arc::new(m7s_rs::flv::FlvTag::new(9, ts, data)));
                tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            }
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(8);
            while tokio::time::Instant::now() < deadline {
                if dest_stream.cache_snapshot().iter().any(|t| t.tag_type == 9) {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            panic!("WebTransport egress did not deliver video tag");
        });
}
