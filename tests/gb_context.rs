use m7s_rs::stream::StreamManager;
use m7s_sdk::EngineContext;

#[test]
fn gb_state_accepts_context() {
    let streams = StreamManager::new(5);
    let ctx = EngineContext::new("gb", "http://127.0.0.1", "rtsp://127.0.0.1")
        .with_streams(streams);
    let state = m7s_plugin_gb28181::GbState::with_context("34020000002000000001", "pass", 0, ctx);
    assert!(state.context().is_some());
    assert_eq!(state.context().unwrap().node_id, "gb");
}
