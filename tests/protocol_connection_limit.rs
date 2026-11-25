use m7s_rs::stream::StreamManager;
use m7s_sdk::EngineContext;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};

async fn rejected_when_full<F>(name: &str, start: F)
where
    F: FnOnce(TcpListener, EngineContext) -> tokio::task::JoinHandle<()>,
{
    eprintln!("testing {name}");
    let streams = StreamManager::new(5);
    let ctx = EngineContext::new("limit", "http://127.0.0.1", "rtsp://127.0.0.1")
        .with_streams(streams);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = start(listener, ctx);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let guard = m7s_sdk::connections::try_acquire().unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut buf = [0u8; 1];
    let n = tokio::time::timeout(std::time::Duration::from_secs(2), s.read(&mut buf))
        .await
        .unwrap()
        .unwrap_or(0);
    assert_eq!(n, 0, "connection should be rejected");
    drop(guard);

    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_ok());
    task.abort();
}

#[test]
fn protocol_accepts_reject_over_limit() {
    unsafe {
        std::env::set_var("M7S_MAX_CONNECTIONS", "1");
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            rejected_when_full("grpc", |l, c| tokio::spawn(async move { let _ = m7s_rs::grpc::run_with_context(l, c).await; })).await;
            rejected_when_full("whep", |l, c| {
                tokio::spawn(async move { let _ = m7s_plugin_webrtc::whep::run_with_context(l, c).await; })
            })
            .await;
            rejected_when_full("whip", |l, c| {
                tokio::spawn(async move { let _ = m7s_plugin_webrtc::whip::run_with_context(l, c).await; })
            })
            .await;
            rejected_when_full("onvif", |l, c| {
                tokio::spawn(async move { let _ = m7s_plugin_onvif::run_with_context(l, c).await; })
            })
            .await;
        });
    unsafe {
        std::env::remove_var("M7S_MAX_CONNECTIONS");
    }
}
