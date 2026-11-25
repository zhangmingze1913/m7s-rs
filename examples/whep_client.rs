use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::ice_transport::ice_candidate_type::RTCIceCandidateType;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;

#[tokio::main]
async fn main() -> Result<()> {
    let _url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:8081/whep/live/ff".to_string());
    let mut media = MediaEngine::default();
    media.register_default_codecs()?;
    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut media)?;
    let mut setting = SettingEngine::default();
    setting.set_nat_1to1_ips(vec!["127.0.0.1".to_string()], RTCIceCandidateType::Host);
    let api = APIBuilder::new()
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .with_setting_engine(setting)
        .build();
    let pc = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await?);

    pc.on_ice_connection_state_change(Box::new(|s| {
        println!("ICE_STATE={s:?}");
        Box::pin(async {})
    }));
    let counter = Arc::new(AtomicUsize::new(0));
    let c = counter.clone();
    pc.on_track(Box::new(move |track, _receiver, _transceiver| {
        let c = c.clone();
        println!("ON_TRACK");
        Box::pin(async move {
            tokio::spawn(async move {
                while let Ok((_pkt, _)) = track.read_rtp().await {
                    c.fetch_add(1, Ordering::Relaxed);
                }
            });
        })
    }));

    let transceiver = pc
        .add_transceiver_from_kind(RTPCodecType::Video, None)
        .await?;
    transceiver
        .set_direction(RTCRtpTransceiverDirection::Recvonly)
        .await;

    let offer = pc.create_offer(None).await?;
    pc.set_local_description(offer).await?;
    let mut gather = pc.gathering_complete_promise().await;
    let _ = gather.recv().await;
    let local = pc.local_description().await.context("local SDP")?;
    println!(
        "OFFER_PT={:?}",
        local
            .sdp
            .lines()
            .filter(|l| l.starts_with("m=video")
                || l.starts_with("a=rtpmap")
                || l.starts_with("a=ssrc")
                || l.starts_with("a=candidate")
                || l.starts_with("a=ice-ufrag")
                || l.starts_with("a=mid")
                || l.starts_with("a=group"))
            .collect::<Vec<_>>()
    );

    let mut socket = TcpStream::connect("127.0.0.1:8081").await?;
    let req = format!(
        "POST /whep/live/ff HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        local.sdp.len(),
        local.sdp
    );
    socket.write_all(req.as_bytes()).await?;
    let mut response = Vec::new();
    socket.read_to_end(&mut response).await?;
    let text = String::from_utf8(response)?;
    let (head, body) = text.split_once("\r\n\r\n").context("WHEP response")?;
    anyhow::ensure!(head.contains(" 201 "), "WHEP status: {head}");
    println!(
        "ANSWER_PT={:?}",
        body.lines()
            .filter(|l| l.starts_with("m=video")
                || l.starts_with("a=rtpmap")
                || l.starts_with("a=ssrc")
                || l.starts_with("a=sendonly")
                || l.starts_with("a=candidate")
                || l.starts_with("a=ice-ufrag")
                || l.starts_with("a=mid")
                || l.starts_with("a=group")
                || l.starts_with("a=recvonly"))
            .collect::<Vec<_>>()
    );
    pc.set_remote_description(RTCSessionDescription::answer(body.to_string())?)
        .await?;

    tokio::time::sleep(Duration::from_secs(12)).await;
    let state = pc.connection_state();
    println!(
        "state={state:?} rtp_packets={}",
        counter.load(Ordering::Relaxed)
    );
    let _ = pc.close().await;
    Ok(())
}
