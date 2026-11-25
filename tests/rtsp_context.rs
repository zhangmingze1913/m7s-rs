use std::sync::Arc;

use m7s_rs::flv::FlvTag;
use m7s_rs::rtsp::run_with_context;
use m7s_rs::stream::StreamManager;
use m7s_sdk::EngineContext;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn rtsp_uses_engine_context() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            let stream = streams.get_or_create("live", "ctx");
            stream.push(Arc::new(FlvTag::new(
                9,
                0,
                vec![0x17, 0, 0, 0, 0, 1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4, 0x68, 1, 2, 3],
            )));
            let ctx = EngineContext::new("n1", "http://127.0.0.1:8180", "rtsp://127.0.0.1:8554")
                .with_streams(streams);
            assert!(ctx.stream("live", "ctx").is_some());
            tokio::spawn(run_with_context(listener, ctx));

            let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            let req = "OPTIONS rtsp://127.0.0.1/live/ctx RTSP/1.0\r\nCSeq: 1\r\n\r\n";
            socket.write_all(req.as_bytes()).await.unwrap();
            let mut buf = [0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            let text = String::from_utf8_lossy(&buf[..n]);
            assert!(text.starts_with("RTSP/1.0 200"), "{text}");
        });
}
