use m7s_rs::stream::StreamManager;
use m7s_sdk::EngineContext;
use tokio::net::{TcpListener, TcpStream};

#[test]
fn onvif_accepts_engine_context() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let streams = StreamManager::new(5);
            let ctx = EngineContext::new("onvif", "http://127.0.0.1", "rtsp://127.0.0.1")
                .with_streams(streams);
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let task = tokio::spawn(m7s_plugin_onvif::run_with_context(listener, ctx));
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            assert!(TcpStream::connect(("127.0.0.1", port)).await.is_ok());
            task.abort();
        });
}
