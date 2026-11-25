//! Minimal RTSP client pull (TCP interleaved H.264) into a StreamManager.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::flv::FlvTag;
use crate::stream::StreamManager;

pub async fn pull(url: &str, streams: Arc<StreamManager>, app: &str, name: &str) -> Result<()> {
    pull_with_retry(url, streams, app, name, u32::MAX).await
}

pub async fn pull_with_retry(
    url: &str,
    streams: Arc<StreamManager>,
    app: &str,
    name: &str,
    max_retries: u32,
) -> Result<()> {
    let mut attempt = 0u32;
    loop {
        let result = pull_once(url, streams.clone(), app, name).await;
        if attempt >= max_retries {
            return result;
        }
        attempt += 1;
        let delay = 200u64.saturating_mul(1u64 << attempt.min(4));
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    }
}

async fn pull_once(url: &str, streams: Arc<StreamManager>, app: &str, name: &str) -> Result<()> {
    let (host, path) = parse_url(url)?;
    let mut socket = TcpStream::connect(&host).await.context("RTSP connect")?;
    let describe = request(&mut socket, "DESCRIBE", &path, 1, "").await?;
    if !describe.0.contains(" 200 ") {
        bail!("DESCRIBE: {}", describe.0.lines().next().unwrap_or(""));
    }
    let (sps, pps) = parse_sprop(&describe.1).with_context(|| format!("SDP missing sprop: {}", describe.1))?;
    let asc = parse_audio_config(&describe.1);
    let setup = request(
        &mut socket,
        "SETUP",
        &format!("{path}/trackID=0"),
        2,
        "Transport: RTP/AVP/TCP;unicast;interleaved=0-1\r\nSession: m7s\r\n",
    )
    .await?;
    if !setup.0.contains(" 200 ") {
        bail!("SETUP: {}", setup.0.lines().next().unwrap_or(""));
    }
    if let Some(asc) = &asc {
        let setup = request(
            &mut socket,
            "SETUP",
            &format!("{path}/trackID=1"),
            3,
            "Transport: RTP/AVP/TCP;unicast;interleaved=2-3\r\nSession: m7s\r\n",
        )
        .await?;
        if !setup.0.contains(" 200 ") {
            bail!("SETUP audio: {}", setup.0.lines().next().unwrap_or(""));
        }
        let mut header = vec![0xAF, 0x00];
        header.extend_from_slice(asc);
        streams.get_or_create(app, name).push(Arc::new(FlvTag::new(8, 0, header)));
    }
    let play = request(&mut socket, "PLAY", &path, 4, "Session: m7s\r\n\r\n").await?;
    if !play.0.contains(" 200 ") {
        bail!("PLAY: {}", play.0.lines().next().unwrap_or(""));
    }

    let stream = streams.get_or_create(app, name);
    let mut header = vec![0x17, 0, 0, 0, 0, 1];
    header.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    header.extend_from_slice(&sps);
    header.push(1);
    header.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    header.extend_from_slice(&pps);
    stream.push(Arc::new(FlvTag::new(9, 0, header)));

    let mut acc = play.2.clone();
    let mut fu: Vec<u8> = Vec::new();
    drain_frames(&mut acc, &mut fu, &stream, &mut socket).await;
    let mut buf = [0u8; 65536];
    loop {
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        acc.extend_from_slice(&buf[..n]);
        drain_frames(&mut acc, &mut fu, &stream, &mut socket).await;
    }
    Ok(())
}

async fn drain_frames(acc: &mut Vec<u8>, fu: &mut Vec<u8>, stream: &crate::stream::Stream, socket: &mut TcpStream) {
    loop {
        if acc.len() < 4 || acc[0] != b'$' {
            if let Some(pos) = acc.iter().position(|b| *b == b'$') {
                acc.drain(..pos);
                continue;
            }
            acc.clear();
            break;
        }
        let len = u16::from_be_bytes([acc[2], acc[3]]) as usize;
        if acc.len() < 4 + len {
            break;
        }
        let channel = acc[1];
        let packet: Vec<u8> = acc[4..4 + len].to_vec();
        acc.drain(..4 + len);
        if channel == 0 {
            if let Some((ts, access)) = depacketize(&packet, fu) {
                let (key, body) = annexb_to_avcc(&access);
                let mut data = vec![if key { 0x17 } else { 0x27 }, 0, 0, 0, 0];
                data.extend_from_slice(&body);
                stream.push(Arc::new(FlvTag::new(9, ts / 90, data)));
            }
        } else if channel == 1 || channel == 3 {
            if let Some(ssrc) = parse_sr(&packet) {
                let rr = build_rr(ssrc);
                let mut frame = vec![b'$', channel, 0, rr.len() as u8];
                frame.extend_from_slice(&rr);
                let _ = socket.write_all(&frame).await;
            }
        } else if channel == 2 {
            for (ts, au) in depacketize_audio(&packet) {
                let mut data = vec![0xAF, 0x01];
                data.extend_from_slice(&au);
                stream.push(Arc::new(FlvTag::new(8, ts / 90, data)));
            }
        }
    }
}

fn parse_url(url: &str) -> Result<(String, String)> {
    let rest = url.strip_prefix("rtsp://").context("rtsp:// required")?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = if authority.contains(':') {
        authority.to_string()
    } else {
        format!("{authority}:554")
    };
    Ok((host, format!("/{path}")))
}

async fn request(
    socket: &mut TcpStream,
    method: &str,
    url: &str,
    cseq: u32,
    extra: &str,
) -> Result<(String, String, Vec<u8>)> {
    let req = format!(
        "{method} {url} RTSP/1.0\r\nCSeq: {cseq}\r\nUser-Agent: m7s-rs-pull\r\n{extra}\r\n"
    );
    socket.write_all(req.as_bytes()).await?;
    let mut head = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&buf[..n]);
        if head.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let pos = head
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("RTSP headers")?;
    let raw = head;
    let head = String::from_utf8_lossy(&raw[..pos]).to_string();
    let mut body = raw[pos + 4..].to_vec();
    let content_len = head
        .lines()
        .find_map(|l| l.strip_prefix("Content-Length:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    while body.len() < content_len {
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
    }
    let payload = if body.len() >= content_len { body[..content_len].to_vec() } else { body.clone() };
    let leftover = if body.len() > content_len { body[content_len..].to_vec() } else { Vec::new() };
    Ok((head, String::from_utf8_lossy(&payload).to_string(), leftover))
}

fn parse_sprop(sdp: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let value = sdp.split("sprop-parameter-sets=").nth(1)?;
    let value = value.split([';', '\r', '\n']).next()?;
    let (sps, pps) = value.split_once(',')?;
    let dec = |s: &str| base64::engine::general_purpose::STANDARD.decode(s.trim()).ok();
    Some((dec(sps)?, dec(pps)?))
}

fn parse_audio_config(sdp: &str) -> Option<Vec<u8>> {
    let value = sdp.split("config=").nth(1)?.split([';', '\r', '\n']).next()?;
    let value = value.trim();
    if value.len() % 2 != 0 {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).ok())
        .collect()
}

fn depacketize_audio(packet: &[u8]) -> Vec<(u32, Vec<u8>)> {
    if packet.len() < 16 {
        return Vec::new();
    }
    let ts = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);
    let offset = 12 + (packet[0] & 0x0f) as usize * 4;
    let payload = &packet[offset..];
    if payload.len() < 4 {
        return Vec::new();
    }
    let header_bits = u16::from_be_bytes([payload[0], payload[1]]) as usize;
    if header_bits == 0 || header_bits & 7 != 0 || header_bits / 8 + 2 > payload.len() {
        return Vec::new();
    }
    let header_len = header_bits / 8;
    let count = header_len / 2;
    let mut pos = 2 + header_len;
    let mut out = Vec::new();
    for i in 0..count {
        let h = 2 + i * 2;
        let size = (u16::from_be_bytes([payload[h], payload[h + 1]]) >> 3) as usize;
        if size == 0 || pos + size > payload.len() {
            break;
        }
        out.push((ts, payload[pos..pos + size].to_vec()));
        pos += size;
    }
    out
}

fn depacketize(packet: &[u8], fu: &mut Vec<u8>) -> Option<(u32, Vec<u8>)> {
    if packet.len() < 13 {
        return None;
    }
    let ts = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);
    let pos = 12 + (packet[0] & 0x0f) as usize * 4;
    let payload = &packet[pos..];
    if payload.is_empty() {
        return None;
    }
    let nal = payload[0] & 0x1f;
    if nal == 28 {
        if payload.len() < 3 {
            return None;
        }
        let start = payload[1] & 0x80 != 0;
        let end = payload[1] & 0x40 != 0;
        if start {
            fu.clear();
            fu.extend_from_slice(&[0, 0, 0, 1, (payload[0] & 0xe0) | (payload[1] & 0x1f)]);
        }
        fu.extend_from_slice(&payload[2..]);
        if end {
            return Some((ts, std::mem::take(fu)));
        }
        None
    } else if (1..=23).contains(&nal) || m7s_codec::is_svc(payload) {
        let mut out = Vec::with_capacity(payload.len() + 4);
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(payload);
        Some((ts, out))
    } else {
        let _ = pos;
        None
    }
}

fn annexb_to_avcc(annexb: &[u8]) -> (bool, Vec<u8>) {
    let mut out = Vec::new();
    let mut i = 0;
    let mut key = false;
    while i + 3 < annexb.len() {
        let start = if annexb[i..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if annexb[i..].starts_with(&[0, 0, 1]) {
            3
        } else {
            i += 1;
            continue;
        };
        let begin = i + start;
        let mut end = annexb.len();
        let mut j = begin;
        while j + 3 < annexb.len() {
            if annexb[j..].starts_with(&[0, 0, 1]) || annexb[j..].starts_with(&[0, 0, 0, 1]) {
                end = j;
                break;
            }
            j += 1;
        }
        if begin < end {
            if annexb[begin] & 0x1f == 5 {
                key = true;
            }
            out.extend_from_slice(&((end - begin) as u32).to_be_bytes());
            out.extend_from_slice(&annexb[begin..end]);
        }
        i = end;
    }
    (key, out)
}

fn parse_sr(packet: &[u8]) -> Option<u32> {
    if packet.len() < 28 || packet[1] != 200 {
        return None;
    }
    Some(u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]))
}

fn build_rr(sender_ssrc: u32) -> [u8; 32] {
    let mut rr = [0u8; 32];
    rr[0] = 0x81;
    rr[1] = 201;
    rr[2..4].copy_from_slice(&7u16.to_be_bytes());
    rr[4..8].copy_from_slice(&0x4d37_0001u32.to_be_bytes());
    rr[8..12].copy_from_slice(&sender_ssrc.to_be_bytes());
    rr
}

#[cfg(test)]
mod svc_tests {
    use super::*;

    #[test]
    fn svc_depacketize_and_filter() {
        let mut rtp = vec![0x80, 0x60, 0, 1, 0, 0, 0, 0, 0, 0, 0, 7];
        rtp.extend_from_slice(&[0x74, 0x45, 0x23, 0x24]);
        let mut fu = Vec::new();
        let (_, access) = depacketize(&rtp, &mut fu).expect("svc nal");
        assert_eq!(m7s_codec::svc_select_layers(&access, 0).len(), 0);
        assert!(!m7s_codec::svc_select_layers(&access, 2).is_empty());
    }
}
