use std::time::Duration;
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::rtp_transceiver::RTCRtpTransceiverInit;

fn api() -> webrtc::api::API {
    let mut media = MediaEngine::default();
    media.register_default_codecs().unwrap();
    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut media).unwrap();
    APIBuilder::new().with_media_engine(media).with_interceptor_registry(registry).build()
}

#[test]
fn offer_answer_ice_with_track() {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let api = api();
        let client = api.new_peer_connection(RTCConfiguration::default()).await.unwrap();
        let server = api.new_peer_connection(RTCConfiguration::default()).await.unwrap();
        for kind in [RTPCodecType::Video, RTPCodecType::Audio] {
            client.add_transceiver_from_kind(kind, Some(RTCRtpTransceiverInit { direction: RTCRtpTransceiverDirection::Recvonly, send_encodings: Vec::new() })).await.unwrap();
        }
        let offer = client.create_offer(None).await.unwrap();
        let mut g = client.gathering_complete_promise().await;
        client.set_local_description(offer).await.unwrap();
        let _ = g.recv().await;
        let offer = client.local_description().await.unwrap();
        server.set_remote_description(offer).await.unwrap();
        let track = std::sync::Arc::new(webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample::new(
            webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability { mime_type: "video/H264".to_string(), clock_rate: 90_000, ..Default::default() },
            "video".to_string(), "m7s".to_string()));
        for tr in server.get_transceivers().await {
            if tr.kind() == RTPCodecType::Video {
                tr.set_sender_track(tr.sender().await, Some(track.clone())).await.unwrap();
                tr.set_direction(RTCRtpTransceiverDirection::Sendonly).await;
            }
        }
        let answer = server.create_answer(None).await.unwrap();
        let mut g = server.gathering_complete_promise().await;
        server.set_local_description(answer).await.unwrap();
        let _ = g.recv().await;
        let answer = server.local_description().await.unwrap();
        client.set_remote_description(answer).await.unwrap();
        let mut ok = false;
        for _ in 0..50 {
            if client.connection_state() == RTCPeerConnectionState::Connected { ok = true; break; }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(ok);
    });
}
