use std::time::Duration;

use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;

#[test]
fn loopback_ice_connects() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).try_init();
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut media = MediaEngine::default();
            media.register_default_codecs().unwrap();
            let mut registry = Registry::new();
            registry = register_default_interceptors(registry, &mut media).unwrap();
            let api = APIBuilder::new()
                .with_media_engine(media)
                .with_interceptor_registry(registry)
                .build();
            let a = api.new_peer_connection(RTCConfiguration::default()).await.unwrap();
            let b = api.new_peer_connection(RTCConfiguration::default()).await.unwrap();
            a.create_data_channel("t", None).await.unwrap();
            let offer = a.create_offer(None).await.unwrap();
            let mut ga = a.gathering_complete_promise().await;
            a.set_local_description(offer).await.unwrap();
            let _ = ga.recv().await;
            let offer = a.local_description().await.unwrap();
            b.set_remote_description(offer).await.unwrap();
            let answer = b.create_answer(None).await.unwrap();
            let mut gb = b.gathering_complete_promise().await;
            b.set_local_description(answer).await.unwrap();
            let _ = gb.recv().await;
            let answer = b.local_description().await.unwrap();
            a.set_remote_description(answer).await.unwrap();
            let mut ok = false;
            for _ in 0..50 {
                if a.connection_state() == RTCPeerConnectionState::Connected {
                    ok = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            eprintln!("loopback connected={ok} {:?}", a.ice_connection_state());
            assert!(ok);
        });
}
