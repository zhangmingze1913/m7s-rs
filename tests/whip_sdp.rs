use std::sync::Arc;
use std::time::Duration;

use m7s_rs::stream::StreamManager;
use m7s_plugin_webrtc::whip;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::ice_transport::ice_candidate_type::RTCIceCandidateType;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::rtp_transceiver::RTCRtpTransceiverInit;

#[test]
fn whip_returns_sdp_answer() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let streams = StreamManager::new(10);
            let stream = streams.get_or_create("live", "whep");
            stream.push(Arc::new(m7s_rs::flv::FlvTag::new(9, 0, vec![0x17, 0, 0, 0, 0, 1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4, 0x68, 1, 2, 3])));
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(whip::run(listener, streams));
            tokio::time::sleep(Duration::from_millis(200)).await;

            let mut media = MediaEngine::default();
            media.register_default_codecs().unwrap();
            let mut registry = Registry::new();
            registry = register_default_interceptors(registry, &mut media).unwrap();
            let mut setting = SettingEngine::default();
            setting.set_nat_1to1_ips(vec!["127.0.0.1".to_string()], RTCIceCandidateType::Host);
            let api = APIBuilder::new()
                .with_media_engine(media)
                .with_interceptor_registry(registry)
                .with_setting_engine(setting)
                .build();
            let pc = api
                .new_peer_connection(RTCConfiguration::default())
                .await
                .unwrap();
            pc.add_transceiver_from_kind(
                RTPCodecType::Video,
                Some(RTCRtpTransceiverInit {
                    direction: RTCRtpTransceiverDirection::Sendonly,
                    send_encodings: Vec::new(),
                }),
            )
            .await
            .unwrap();
            let offer = pc.create_offer(None).await.unwrap();
            pc.set_local_description(offer.clone()).await.unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
            while pc.ice_gathering_state() != webrtc::ice_transport::ice_gathering_state::RTCIceGatheringState::Complete
                && tokio::time::Instant::now() < deadline
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let local = pc.local_description().await.unwrap();

            let (status, body) = post_sdp(port, &local.sdp).await;
            assert_eq!(status, 201, "{}", String::from_utf8_lossy(&body));
            let answer = String::from_utf8_lossy(&body);
            assert!(answer.contains("v=0"), "{answer}");
            assert!(answer.contains("m=video"), "{answer}");
        });
}

async fn post_sdp(port: u16, sdp: &str) -> (u16, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = format!(
        "POST /whip/live/whip HTTP/1.1\r\nHost: t\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{}",
        sdp.len(),
        sdp
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| buf[p + 4..].to_vec())
        .unwrap_or_default();
    (status, body)
}
