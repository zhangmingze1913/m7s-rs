use m7s_rs::stream::StreamManager;
use m7s_sdk::EngineContext;
use tokio::net::{TcpListener, TcpStream};

#[test]
fn whep_whip_accept_engine_context() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let streams = StreamManager::new(5);
            let ctx = EngineContext::new("ctx", "http://127.0.0.1", "rtsp://127.0.0.1")
                .with_streams(streams.clone());

            let whep = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let whep_port = whep.local_addr().unwrap().port();
            let whep_task = tokio::spawn(m7s_plugin_webrtc::whep::run_with_context(whep, ctx.clone()));
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            assert!(TcpStream::connect(("127.0.0.1", whep_port)).await.is_ok());
            whep_task.abort();

            let whip = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let whip_port = whip.local_addr().unwrap().port();
            let whip_task = tokio::spawn(m7s_plugin_webrtc::whip::run_with_context(whip, ctx));
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            assert!(TcpStream::connect(("127.0.0.1", whip_port)).await.is_ok());
            whip_task.abort();
        });
}
