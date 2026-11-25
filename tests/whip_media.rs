use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use m7s_plugin_webrtc::whip;
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

#[test]
fn whip_media_plane() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let streams = StreamManager::new(10);
            let stream = streams.get_or_create("live", "whip-media");
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(whip::run(listener, streams));
            tokio::time::sleep(Duration::from_millis(200)).await;

            let mut media = MediaEngine::default();
            media.register_default_codecs().unwrap();
            let mut registry = Registry::new();
            registry = register_default_interceptors(registry, &mut media).unwrap();
            let api = APIBuilder::new()
                .with_media_engine(media)
                .with_interceptor_registry(registry)
                .build();
            let pc = api
                .new_peer_connection(RTCConfiguration::default())
                .await
                .unwrap();
            let track = Arc::new(TrackLocalStaticSample::new(
                RTCRtpCodecCapability {
                    mime_type: "video/H264".to_string(),
                    clock_rate: 90_000,
                    sdp_fmtp_line: "packetization-mode=1;profile-level-id=42001e".to_string(),
                    ..Default::default()
                },
                "video".to_string(),
                "whip".to_string(),
            ));
            pc.add_track(track.clone()).await.unwrap();
            pc.get_transceivers().await.first().unwrap()
                .set_direction(RTCRtpTransceiverDirection::Sendonly)
                .await;

            let offer = pc.create_offer(None).await.unwrap();
            let mut gather = pc.gathering_complete_promise().await;
            pc.set_local_description(offer).await.unwrap();
            let _ = gather.recv().await;
            let local = pc.local_description().await.unwrap();
            let (status, body) = post_sdp(port, &local.sdp).await;
            assert_eq!(status, 201, "{}", String::from_utf8_lossy(&body));
            let answer = String::from_utf8_lossy(&body);
            assert!(answer.contains("m=video"), "{answer}");
            pc.set_remote_description(RTCSessionDescription::answer(answer.to_string()).unwrap())
                .await
                .unwrap();

            let mut connected = false;
            for _ in 0..50 {
                if pc.connection_state() == RTCPeerConnectionState::Connected {
                    connected = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            assert!(connected, "WHIP ICE not connected");

            let annexb = vec![
                0, 0, 0, 1, 0x67, 0x42, 0, 0x1e, 0xE1, 0x01, 0x40, 0, 0, 0, 1, 0x68, 0xCE, 0x3C, 0x80,
                0, 0, 0, 1, 0x65, 0x88, 0x84, 0x00,
            ];
            for n in 0..40u8 {
                let mut data = annexb.clone();
                data.push(n);
                let sample = Sample {
                    data: Bytes::from(data),
                    duration: Duration::from_millis(40),
                    ..Default::default()
                };
                let _ = track.write_sample(&sample).await;
                tokio::time::sleep(Duration::from_millis(40)).await;
                if stream.cache_snapshot().iter().any(|t| t.tag_type == 9) {
                    return;
                }
            }
            panic!("no WHIP video tag");
        });
}

async fn post_sdp(port: u16, sdp: &str) -> (u16, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = format!(
        "POST /whip/live/whip-media HTTP/1.1\r\nHost: t\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{}",
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
