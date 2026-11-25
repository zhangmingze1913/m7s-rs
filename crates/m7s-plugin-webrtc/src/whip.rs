//! Minimal WHIP ingest: `POST /whip/<app>/<name>` with an SDP offer.
//! Receives H.264 RTP and pushes FLV tags into the shared stream.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use rtp::codecs::h264::H264Packet;
use rtp::packetizer::Depacketizer;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

use m7s_sdk::StreamApi;

pub async fn run_with_context(listener: TcpListener, ctx: m7s_sdk::EngineContext) -> Result<()> {
    let streams = ctx.stream_api().ok_or_else(|| anyhow::anyhow!("EngineContext missing streams"))?;
    run(listener, streams).await
}

pub async fn run(listener: TcpListener, streams: Arc<dyn StreamApi>) -> Result<()> {
    log::info!("WHIP listening on {}", listener.local_addr()?);
    loop {
        let (socket, peer) = listener.accept().await?;
        let Some(guard) = m7s_sdk::connections::try_acquire() else {
            drop(socket);
            continue;
        };
        let streams = streams.clone();
        tokio::spawn(async move {
            let _guard = guard;
            if let Err(e) = handle(socket, streams).await {
                log::debug!("WHIP {peer}: {e:#}");
            }
        });
    }
}

async fn handle(mut socket: TcpStream, streams: Arc<dyn StreamApi>) -> Result<()> {
    let mut head = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            bail!("WHIP closed");
        }
        head.extend_from_slice(&buf[..n]);
        if head.windows(4).any(|w| w == b"\r\n\r\n") || head.len() > 64 * 1024 {
            break;
        }
    }
    let pos = head
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("WHIP headers")?;
    let headers = String::from_utf8_lossy(&head[..pos]).to_string();
    let mut body = head[pos + 4..].to_vec();
    let mut lines = headers.lines();
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");
    if method != "POST" {
        return respond(&mut socket, 405, "method not allowed").await;
    }
    let mut content_len = 0usize;
    for line in lines {
        if let Some(v) = line.strip_prefix("Content-Length:") {
            content_len = v.trim().parse().unwrap_or(0);
        }
    }
    while body.len() < content_len {
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
    }
    let rest = path.strip_prefix("/whip/").unwrap_or("");
    let (app, name) = rest.split_once('/').unwrap_or(("live", "test"));
    let Some(stream) = streams.stream(app, name) else {
        anyhow::bail!("stream seat limit exceeded");
    };
    let offer = String::from_utf8(body)?;

    let key = format!("{app}/{name}");
    let mut media = MediaEngine::default();
    media.register_default_codecs()?;
    media.register_header_extension(
        webrtc::rtp_transceiver::rtp_codec::RTCRtpHeaderExtensionCapability {
            uri: "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01".to_string(),
        },
        webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Video,
        None,
    )?;
    let api = APIBuilder::new()
        .with_media_engine(media)
        .with_interceptor_registry(crate::registry(&key))
        .build();
    let pc = Arc::new(api.new_peer_connection(crate::rtc_config()).await?);

    let ingest = stream.clone();
    let selected = crate::simulcast_rid(&offer);
    let key_stats = key.clone();
    pc.on_track(Box::new(move |track, _receiver, _transceiver| {
        let rid = track.rid().to_string();
        let key = key.clone();
        let selected = selected.clone();
        let stream = ingest.clone();
        Box::pin(async move {
            tokio::spawn(async move {
                let mut depack = H264Packet::default();
                depack.is_avc = true;
                let mut base = None;
                let mut sps: Option<Vec<u8>> = None;
                let mut pps: Option<Vec<u8>> = None;
                let mut config_sent = false;
                let mut count = 0u64;
                while let Ok((pkt, _)) = track.read_rtp().await {
                    if !rid.is_empty() {
                        let eff = crate::rid_for(&key, selected.as_ref());
                        if eff.as_deref() != Some(rid.as_str()) {
                            continue;
                        }
                    }
                    count += 1;
                    if count % 50 == 0 {
                        log::info!("WHIP RTP count={count} payload={}", pkt.payload.len());
                    }
                    let ts = *base.get_or_insert(pkt.header.timestamp);
                    let ms = pkt.header.timestamp.wrapping_sub(ts) / 90;
                    let Ok(avcc) = depack.depacketize(&pkt.payload) else {
                        continue;
                    };
                    if avcc.is_empty() {
                        continue;
                    }
                    let mut pos = 0usize;
                    let mut key = false;
                    while pos + 4 <= avcc.len() {
                        let len =
                            u32::from_be_bytes(avcc[pos..pos + 4].try_into().unwrap()) as usize;
                        pos += 4;
                        if len == 0 || pos + len > avcc.len() {
                            break;
                        }
                        let nalu = &avcc[pos..pos + len];
                        match nalu[0] & 0x1f {
                            7 => sps = Some(nalu.to_vec()),
                            8 => pps = Some(nalu.to_vec()),
                            5 => key = true,
                            _ => {}
                        }
                        pos += len;
                    }
                    if !config_sent {
                        if let (Some(s), Some(p)) = (&sps, &pps) {
                            stream.push_tag(9, 0, avc_config(s, p));
                            config_sent = true;
                        }
                    }
                    if !config_sent {
                        continue;
                    }
                    let mut data = vec![if key { 0x17 } else { 0x27 }, 1, 0, 0, 0];
                    data.extend_from_slice(&avcc);
                    stream.push_tag(9, ms, data);
                }
            });
        })
    }));

    let pc_stats = pc.clone();
    tokio::spawn(async move {
        let mut last = 0u64;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let stats = pc_stats.get_stats().await;
            let nacks: u64 = stats.reports.values().filter_map(|r| {
                if let webrtc::stats::StatsReportType::InboundRTP(s) = r { Some(s.nack_count) } else { None }
            }).sum();
            let delta = nacks.saturating_sub(last);
            last = nacks;
            let cur = crate::rid_for(&key_stats, None).unwrap_or_default();
            let next = crate::auto_rid(delta, 0.0, &cur);
            if next != cur { crate::set_rid(&key_stats, &next); }
        }
    });

    pc.set_remote_description(RTCSessionDescription::offer(offer)?)
        .await?;
    let answer = pc.create_answer(None).await?;
    let mut gather = pc.gathering_complete_promise().await;
    pc.set_local_description(answer).await?;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), gather.recv()).await;
    let local = pc.local_description().await.context("WHIP answer")?;
    respond(&mut socket, 201, &local.sdp).await
}

fn avc_config(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut out = vec![0x17, 0, 0, 0, 0, 1];
    out.push(sps.get(1).copied().unwrap_or(0x42));
    out.push(sps.get(2).copied().unwrap_or(0));
    out.push(sps.get(3).copied().unwrap_or(0x1f));
    out.push(0xff);
    out.push(0xe1);
    out.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    out.extend_from_slice(sps);
    out.push(1);
    out.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    out.extend_from_slice(pps);
    out
}

async fn respond(socket: &mut TcpStream, status: u16, body: &str) -> Result<()> {
    let reason = if status == 201 { "Created" } else { "OK" };
    let out = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(out.as_bytes()).await?;
    socket.flush().await?;
    Ok(())
}
