use std::sync::Arc;

use m7s_rs::flv::FlvTag;
use m7s_rs::rtsp;
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn rtsp_runtime_plugin_toggle() {
    let cfg = std::env::temp_dir().join(format!("m7s-rtsp-toggle-{}.json", std::process::id()));
    unsafe { std::env::set_var("M7S_PLUGIN_CONFIG", &cfg) };
    std::fs::write(&cfg, b"[]").unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            let stream = streams.get_or_create("live", "toggle");
            let mut seq = vec![0x17, 0, 0, 0, 0, 0x01, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4, 0x68, 1, 2, 3];
            seq.extend_from_slice(&[0x17, 1, 0, 0, 0, 0, 0, 0, 4, 0x65, 1, 2, 3]);
            stream.push(Arc::new(FlvTag::new(9, 0, seq)));
            tokio::spawn(rtsp::run(listener, streams));

            assert!(options(port).await.contains("RTSP/1.0 200"), "enabled RTSP should answer");

            std::fs::write(&cfg, br#"["rtsp"]"#).unwrap();
            let disabled = options(port).await;
            assert!(disabled.is_empty() || !disabled.contains("200"), "disabled RTSP should not answer: {disabled}");

            std::fs::write(&cfg, b"[]").unwrap();
            assert!(options(port).await.contains("RTSP/1.0 200"), "re-enabled RTSP should answer");
        });
    unsafe { std::env::remove_var("M7S_PLUGIN_CONFIG") };
    let _ = std::fs::remove_file(cfg);
}

async fn options(port: u16) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = "OPTIONS rtsp://127.0.0.1/live/toggle RTSP/1.0\r\nCSeq: 1\r\n\r\n";
    if s.write_all(req.as_bytes()).await.is_err() {
        return String::new();
    }
    let mut buf = [0u8; 2048];
    match tokio::time::timeout(std::time::Duration::from_secs(2), s.read(&mut buf)).await {
        Ok(Ok(n)) => String::from_utf8_lossy(&buf[..n]).to_string(),
        _ => String::new(),
    }
}
