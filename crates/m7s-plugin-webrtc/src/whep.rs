//! Minimal WHEP (WebRTC-HTTP Egress Protocol) endpoint.
//!
//! `POST /whep/<app>/<name>` with an SDP offer returns an SDP answer.
//! H.264 FLV tags are repacketized to Annex B and written to a WebRTC track.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::ice_transport::ice_candidate_type::RTCIceCandidateType;
use webrtc::media::Sample;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

use m7s_sdk::{StreamApi, TagStream};

pub async fn run_with_context(listener: TcpListener, ctx: m7s_sdk::EngineContext) -> Result<()> {
    let streams = ctx.stream_api().ok_or_else(|| anyhow::anyhow!("EngineContext missing streams"))?;
    run(listener, streams).await
}

pub async fn run(listener: TcpListener, streams: Arc<dyn StreamApi>) -> Result<()> {
    log::info!("WHEP listening on {}", listener.local_addr()?);
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
                log::debug!("WHEP {peer}: {e:#}");
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
            bail!("WHEP closed");
        }
        head.extend_from_slice(&buf[..n]);
        if head.windows(4).any(|w| w == b"\r\n\r\n") || head.len() > 64 * 1024 {
            break;
        }
    }
    let pos = head
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("WHEP headers")?;
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
    let rest = path.strip_prefix("/whep/").unwrap_or("");
    let (app, name) = rest.split_once('/').unwrap_or(("live", "test"));
    let Some(stream) = streams.stream(app, name) else {
        return respond(&mut socket, 404, "stream not found").await;
    };
    let offer = String::from_utf8(body)?;
    let cache: Vec<Arc<m7s_codec::FlvTag>> = stream.snapshot().into_iter().map(|t| Arc::new(m7s_codec::FlvTag::new(t.tag_type, t.timestamp, t.data))).collect();
    let mut rx = stream.tap();
    let (sps, pps) = m7s_codec::avc_config(&cache);
    let has_video = offer.contains("m=video");
    let has_audio = offer.contains("m=audio");
    let audio_asc = m7s_codec::audio_config(&cache);
    let fmtp = if sps.is_empty() {
        String::new()
    } else {
        format!(
            "packetization-mode=1;sprop-parameter-sets={},{}",
            STANDARD.encode(&sps),
            STANDARD.encode(&pps)
        )
    };

    let key = format!("{app}/{name}");
    let mut media = MediaEngine::default();
    media.register_default_codecs()?;
    for typ in [webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Video, webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Audio] {
        media.register_header_extension(
            webrtc::rtp_transceiver::rtp_codec::RTCRtpHeaderExtensionCapability {
                uri: "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01".to_string(),
            },
            typ,
            None,
        )?;
    }
    let mut setting = SettingEngine::default();
    setting.set_nat_1to1_ips(vec!["127.0.0.1".to_string()], RTCIceCandidateType::Host);
    let api = APIBuilder::new()
        .with_media_engine(media)
        .with_interceptor_registry(crate::registry(&key))
        .with_setting_engine(setting)
        .build();
    let pc = Arc::new(api.new_peer_connection(crate::rtc_config()).await?);
    let track: Arc<TrackLocalStaticSample> = Arc::new(TrackLocalStaticSample::new(
        RTCRtpCodecCapability {
            mime_type: "video/H264".to_string(),
            clock_rate: 90_000,
            sdp_fmtp_line: fmtp.clone(),
            ..Default::default()
        },
        "video".to_string(),
        "m7s".to_string(),
    ));
    let audio_track: Option<Arc<TrackLocalStaticSample>> = if !has_audio || audio_asc.is_empty() {
        None
    } else {
        Some(Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: "audio/PCMU".to_string(),
                clock_rate: 8_000,
                channels: 1,
                ..Default::default()
            },
            "audio".to_string(),
            "m7s".to_string(),
        )))
    };
    log::info!("WHEP offer candidates={}", offer.matches("a=candidate").count());
    pc.set_remote_description(RTCSessionDescription::offer(offer)?)
        .await?;
    for transceiver in pc.get_transceivers().await {
        match transceiver.kind() {
            webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Video if has_video => {
                transceiver
                    .set_sender_track(transceiver.sender().await, Some(track.clone()))
                    .await?;
                transceiver
                    .set_direction(RTCRtpTransceiverDirection::Sendonly)
                    .await;
            }
            webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Audio => {
                if let Some(audio) = &audio_track {
                    transceiver
                        .set_sender_track(transceiver.sender().await, Some(audio.clone()))
                        .await?;
                    transceiver
                        .set_direction(RTCRtpTransceiverDirection::Sendonly)
                        .await;
                }
            }
            _ => {}
        }
    }
    let answer = pc.create_answer(None).await?;
    let mut gather = pc.gathering_complete_promise().await;
    pc.set_local_description(answer).await?;
    let _ = tokio::time::timeout(Duration::from_secs(3), gather.recv()).await;
    let local = pc.local_description().await.context("WHEP answer")?;
    respond(&mut socket, 201, &local.sdp).await?;
    for _ in 0..200 {
        if pc.connection_state() == RTCPeerConnectionState::Connected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if let Some(audio) = audio_track {
        let audio_stream = stream.clone();
        tokio::spawn(audio_feed(audio_stream, audio, audio_asc));
    }
    let pc = pc.clone();
    tokio::spawn(async move {
        let mut seq = 0u16;
        for tag in cache {
            if let Err(e) = feed_tag(&track, &tag, &mut seq, &sps, &pps).await {
                log::error!("WHEP video feed: {e}");
                stream.leave();
            }
        }
        loop {
            match rx.recv().await {
                Ok(tag) => {
                    let tag = m7s_codec::FlvTag::new(tag.tag_type, tag.timestamp, tag.data.clone());
                    if let Err(e) = feed_tag(&track, &tag, &mut seq, &sps, &pps).await {
                        log::error!("WHEP video feed: {e}");
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
        stream.leave();
        let _ = pc.close().await;
    });
    Ok(())
}

async fn feed_tag(
    track: &TrackLocalStaticSample,
    tag: &m7s_codec::FlvTag,
    seq: &mut u16,
    sps: &[u8],
    pps: &[u8],
) -> Result<()> {
    if tag.tag_type != 9 || tag.data.len() < 5 || (tag.data[0] & 0x0f) != 7 || tag.data[1] != 1 {
        return Ok(());
    }
    let mut annexb = Vec::new();
    if (tag.data[0] >> 4) == 1 && !sps.is_empty() {
        annexb.extend_from_slice(&[0, 0, 0, 1]);
        annexb.extend_from_slice(sps);
        annexb.extend_from_slice(&[0, 0, 0, 1]);
        annexb.extend_from_slice(pps);
    }
    let mut pos = 5;
    while pos + 4 <= tag.data.len() {
        let len = u32::from_be_bytes(tag.data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if len == 0 || pos + len > tag.data.len() {
            break;
        }
        annexb.extend_from_slice(&[0, 0, 0, 1]);
        annexb.extend_from_slice(&tag.data[pos..pos + len]);
        pos += len;
    }
    if annexb.is_empty() {
        return Ok(());
    }
    *seq = seq.wrapping_add(1);
    if *seq % 50 == 0 {
        log::info!("WHEP H264 samples={seq}");
    }
    track
        .write_sample(&Sample {
            data: Bytes::from(annexb),
            duration: Duration::from_millis(40),
            ..Default::default()
        })
        .await?;
    Ok(())
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

async fn audio_feed(
    stream: Arc<dyn TagStream>,
    track: Arc<TrackLocalStaticSample>,
    asc: Vec<u8>,
) {
    let cache: Vec<Arc<m7s_codec::FlvTag>> = stream.snapshot().into_iter().map(|t| Arc::new(m7s_codec::FlvTag::new(t.tag_type, t.timestamp, t.data))).collect();
    let mut rx = stream.tap();
    let ffmpeg = std::env::var("M7S_FFMPEG").unwrap_or_else(|_| {
        if std::path::Path::new("node_modules/ffmpeg-static/ffmpeg.exe").exists() {
            "node_modules/ffmpeg-static/ffmpeg.exe".to_string()
        } else {
            "ffmpeg".to_string()
        }
    });
    let mut child = match Command::new(&ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "aac",
            "-i",
            "pipe:0",
            "-f",
            "s16le",
            "-ac",
            "1",
            "-ar",
            "8000",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            log::warn!("WHEP audio ffmpeg unavailable: {e}");
            stream.leave();
            return;
        }
    };
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let cfg = asc.clone();
    tokio::spawn(async move {
        for tag in cache {
            if tag.tag_type == 8
                && tag.data.len() > 2
                && tag.data[1] == 1
                && write_adts(&mut stdin, &cfg, &tag.data[2..]).await.is_err()
            {
                return;
            }
        }
        loop {
            match rx.recv().await {
                Ok(tag) => {
                    if tag.tag_type == 8
                        && tag.data.len() > 2
                        && tag.data[1] == 1
                        && write_adts(&mut stdin, &cfg, &tag.data[2..]).await.is_err()
                    {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });

    let mut pcm = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = match stdout.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        pcm.extend_from_slice(&buf[..n]);
        while pcm.len() >= 320 {
            let mut out = [0u8; 160];
            for (i, chunk) in pcm.chunks_exact(2).take(160).enumerate() {
                out[i] = mulaw(i16::from_le_bytes([chunk[0], chunk[1]]));
            }
            pcm.drain(..320);
            if track
                .write_sample(&Sample {
                    data: Bytes::copy_from_slice(&out),
                    duration: Duration::from_millis(20),
                    ..Default::default()
                })
                .await
                .is_err()
            {
                break;
            }
        }
    }
    let _ = child.kill().await;
    stream.leave();
}

async fn write_adts<W: tokio::io::AsyncWrite + Unpin>(
    w: &mut W,
    asc: &[u8],
    raw: &[u8],
) -> Result<()> {
    if asc.len() < 2 {
        return Ok(());
    }
    let profile = (asc[0] >> 6) & 0x03;
    let freq = asc[0] & 0x07;
    let chan = ((asc[0] & 0x01) << 2) | (asc[1] >> 6);
    let len = raw.len() + 7;
    let hdr = [
        0xff,
        0xf1,
        (profile << 6) | (freq << 2) | (chan >> 2),
        ((chan & 3) << 6) | ((len >> 11) as u8 & 0x03),
        (len >> 3) as u8,
        (((len & 7) as u8) << 5) | 0x1f,
        0xfc,
    ];
    w.write_all(&hdr).await?;
    w.write_all(raw).await?;
    w.flush().await?;
    Ok(())
}

fn mulaw(sample: i16) -> u8 {
    let mut x = sample as i32;
    let sign = if x < 0 {
        x = -x;
        0x80
    } else {
        0
    };
    if x > 32635 {
        x = 32635;
    }
    x += 0x84;
    let mut exponent = 7;
    let mut mask = 0x4000;
    while exponent > 0 && (x & mask) == 0 {
        exponent -= 1;
        mask >>= 1;
    }
    let mantissa = (x >> (exponent + 3)) & 0x0f;
    (sign | (exponent << 4) | mantissa) as u8
}
