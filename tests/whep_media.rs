// NOTE: the client must NOT set `nat_1to1_ips`; rewriting candidates to 127.0.0.1 breaks WHEP ICE pairing.
use std::sync::Arc;
use std::time::Duration;

use m7s_rs::stream::StreamManager;
use m7s_plugin_webrtc::whep;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::rtp_transceiver::RTCRtpTransceiverInit;

#[test]
fn whep_media_plane() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).try_init();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let streams = StreamManager::new(10);
            let stream = streams.get_or_create("live", "whep");
            stream.push(Arc::new(m7s_rs::flv::FlvTag::new(9, 0, vec![0x17, 0, 0, 0, 0, 1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4, 0x68, 1, 2, 3])));
            stream.push(Arc::new(m7s_rs::flv::FlvTag::new(8, 0, vec![0xAF, 0x00, 0x12, 0x10])));
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(whep::run(listener, streams));
            tokio::time::sleep(Duration::from_millis(200)).await;

            let mut media = MediaEngine::default();
            media.register_default_codecs().unwrap();
            for typ in [webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Video, webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Audio] {
                media.register_header_extension(webrtc::rtp_transceiver::rtp_codec::RTCRtpHeaderExtensionCapability { uri: "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01".to_string() }, typ, None).unwrap();
            }
            let mut registry = Registry::new();
            registry = register_default_interceptors(registry, &mut media).unwrap();
            let setting = SettingEngine::default();
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
                    direction: RTCRtpTransceiverDirection::Recvonly,
                    send_encodings: Vec::new(),
                }),
            )
            .await
            .unwrap();
            pc.add_transceiver_from_kind(
                RTPCodecType::Audio,
                Some(RTCRtpTransceiverInit {
                    direction: RTCRtpTransceiverDirection::Recvonly,
                    send_encodings: Vec::new(),
                }),
            )
            .await
            .unwrap();
            let offer = pc.create_offer(None).await.unwrap();
            let mut gather = pc.gathering_complete_promise().await;
            pc.set_local_description(offer).await.unwrap();
            let _ = gather.recv().await;
            let local = pc.local_description().await.unwrap();
            let (rtp_tx, mut rtp_rx) = tokio::sync::mpsc::channel::<u8>(4);
            pc.on_track(Box::new(move |track, _receiver, _transceiver| {
                let rtp_tx = rtp_tx.clone();
                Box::pin(async move {
                    if let Ok((pkt, _)) = track.read_rtp().await {
                        let _ = rtp_tx.send(pkt.header.payload_type).await;
                    }
                })
            }));

            let (status, body) = post_sdp(port, &local.sdp).await;
            assert_eq!(status, 201, "{}", String::from_utf8_lossy(&body));
            let answer = String::from_utf8_lossy(&body);
            assert!(answer.contains("v=0"), "{answer}");
            assert!(answer.contains("m=video"), "{answer}");
            assert!(answer.contains("m=audio"), "{answer}");
            assert!(answer.contains("PCMU/8000"), "{answer}");
            assert!(answer.contains("a=candidate"), "missing ICE candidate: {answer}");
            assert!(answer.contains("a=ice-ufrag"), "missing ice-ufrag: {answer}");
            assert!(answer.contains("transport-cc"), "missing transport-cc: {answer}");
            pc.set_remote_description(webrtc::peer_connection::sdp::session_description::RTCSessionDescription::answer(answer.to_string()).unwrap()).await.unwrap();
            let mut ok = false;
            for _ in 0..50 {
                if pc.connection_state() == webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Connected { ok = true; break; }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            eprintln!("connected={ok} ice={:?}", pc.ice_connection_state());
            assert!(ok, "WHEP not connected");
            let s = stream.clone();
            tokio::spawn(async move {
                for ts in (100u32..5000).step_by(100) {
                    s.push(Arc::new(m7s_rs::flv::FlvTag::new(9, ts, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 4, 0x65, 1, 2, 3])));
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            });
            let got = tokio::time::timeout(Duration::from_secs(8), rtp_rx.recv()).await;
            assert!(got.is_ok() && got.unwrap().is_some(), "no RTP media");
        });
}

async fn post_sdp(port: u16, sdp: &str) -> (u16, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = format!(
        "POST /whep/live/whep HTTP/1.1\r\nHost: t\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{}",
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
