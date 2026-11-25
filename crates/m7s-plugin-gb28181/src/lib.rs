#![allow(clippy::collapsible_if)]
#![allow(clippy::items_after_test_module)]
//! GB28181 SIP registrar (first step).
//!
//! Implements REGISTER and MESSAGE keepalive over UDP. Catalog/INVITE and
//! RTP/PS media are not implemented yet; this is a real SIP registrar, not a
//! stub, but it is not full GB28181.

pub mod ps;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

use anyhow::Result;
use serde_json::{Value, json};
use tokio::net::UdpSocket;

use m7s_sdk::StreamApi;

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}


#[derive(Debug, Clone)]
pub struct Device {
    pub id: String,
    pub contact: String,
    pub expires_ms: i64,
    pub last_seen_ms: i64,
}

#[derive(Default)]
struct GbSession {
    demux: PsDemuxer,
    sps: Vec<u8>,
    pps: Vec<u8>,
    ts: u32,
    config_sent: bool,
    asc: Vec<u8>,
    audio_ts: u32,
    last_seq: Option<u16>,
    #[allow(dead_code)]
    lost: u64,
    pending: BTreeMap<u16, Vec<u8>>,
}

pub struct GbState {
    domain: String,
    password: String,
    media_port: u16,
    devices: RwLock<HashMap<String, Device>>,
    nonces: RwLock<HashMap<String, i64>>,
    media: RwLock<MediaStats>,
    streams: Arc<dyn StreamApi>,
    sessions: RwLock<HashMap<String, GbSession>>,
    ptz: RwLock<Option<String>>,
    playback: RwLock<PlaybackState>,
    last_invite: RwLock<String>,
    playback_target: RwLock<Option<std::net::SocketAddr>>,
    context: Option<m7s_sdk::EngineContext>,
}

impl GbState {
    pub fn new(
        domain: &str,
        password: &str,
        media_port: u16,
        streams: Arc<dyn StreamApi>,
    ) -> Arc<Self> {
        Arc::new(Self {
            domain: domain.to_string(),
            password: password.to_string(),
            media_port,
            devices: RwLock::new(HashMap::new()),
            nonces: RwLock::new(HashMap::new()),
            media: RwLock::new(MediaStats::default()),
            streams,
            sessions: RwLock::new(HashMap::new()),
            ptz: RwLock::new(None),
            playback: RwLock::new(PlaybackState::default()),
            last_invite: RwLock::new(String::new()),
            playback_target: RwLock::new(None),
            context: None,
        })
    }

    fn fresh_nonce(&self) -> String {
        let nonce = format!("{:x}", now_ms());
        self.nonces
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(nonce.clone(), now_ms() + 60_000);
        nonce
    }

    pub fn playback(&self) -> PlaybackState {
        self.playback.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn last_ptz(&self) -> Option<String> {
        self.ptz
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn playback_target(&self) -> Option<std::net::SocketAddr> {
        *self.playback_target.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn last_invite(&self) -> String {
        self.last_invite.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn device_count(&self) -> usize {
        self.devices.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn devices_json(&self) -> Value {
        json!({
            "devices": self.devices.read().unwrap_or_else(|e| e.into_inner()).values().map(|d| json!({
                "id": d.id,
                "contact": d.contact,
                "expires_ms": d.expires_ms,
            })).collect::<Vec<_>>()
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackState {
    pub paused: bool,
    pub speed: f32,
    pub range: String,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self { paused: false, speed: 1.0, range: String::new() }
    }
}

#[derive(Debug, Default, Clone)]
pub struct MediaStats {
    pub rtp_packets: u64,
    pub ps_packets: u64,
    pub bytes: u64,
}

pub fn expire_devices(state: &GbState) {
    let now = now_ms();
    state
        .devices
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|_, d| d.expires_ms > now);
}

pub async fn expire_loop(state: Arc<GbState>) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        expire_devices(&state);
    }
}
pub async fn run_media(socket: UdpSocket, state: Arc<GbState>) -> Result<()> {
    log::info!("GB28181 RTP listening on {}", socket.local_addr()?);
    let mut buf = [0u8; 65536];
    loop {
        let (n, peer) = socket.recv_from(&mut buf).await?;
        let seq = rtp_seq(&buf[..n]);
        let Some(payload) = rtp_payload(&buf[..n]) else {
            continue;
        };
        {
            let mut stats = state.media.write().unwrap_or_else(|e| e.into_inner());
            stats.rtp_packets += 1;
            stats.bytes += payload.len() as u64;
            if payload.windows(4).any(|w| w == [0, 0, 1, 0xba]) {
                stats.ps_packets += 1;
            }
        }
        let key = peer.ip().to_string();
        let Some(stream) = state.streams.stream("gb", &key) else {
            return Ok(());
        };
        let mut sessions = state.sessions.write().unwrap_or_else(|e| e.into_inner());
        let session = sessions.entry(key).or_default();
        if let Some(seq) = seq {
            let expected = session.last_seq.map(|s| s.wrapping_add(1));
            if session.last_seq.is_none() || expected == Some(seq) {
                session.last_seq = Some(seq);
                process_payload(&stream, session, payload);
                loop {
                    let next = session.last_seq.unwrap_or(seq).wrapping_add(1);
                    let Some(data) = session.pending.remove(&next) else {
                        break;
                    };
                    session.last_seq = Some(next);
                    process_payload(&stream, session, &data);
                }
            } else if session.pending.len() < 64 {
                session.pending.insert(seq, payload.to_vec());
            } else {
                process_payload(&stream, session, payload);
            }
        } else {
            process_payload(&stream, session, payload);
        }
    }
}

fn process_payload(stream: &Arc<dyn m7s_sdk::TagStream>, session: &mut GbSession, payload: &[u8]) {
    session.demux.push(payload);
    while let Some((code, pes, pts, dts)) = session.demux.next_pes() {
        if let Some(pts) = pts {
            session.ts = pts;
            session.audio_ts = pts;
        }
        if code == 0xc0 {
            if let Some((raw, asc)) = adts_frame(&pes) {
                if session.asc != asc {
                    session.asc = asc.clone();
                    let mut cfg = vec![0xaf, 0];
                    cfg.extend_from_slice(&asc);
                    stream.push_tag(8, session.audio_ts, cfg);
                }
                let mut data = vec![0xaf, 1];
                data.extend_from_slice(&raw);
                stream.push_tag(8, session.audio_ts, data);
                session.audio_ts = session.audio_ts.wrapping_add(23);
            }
            continue;
        }
        if code != 0xe0 {
            continue;
        }
        let nalus = annexb_nalus(&pes);
        for nalu in &nalus {
            if nalu.is_empty() {
                continue;
            }
            match nalu[0] & 0x1f {
                7 => {
                    session.sps = nalu.to_vec();
                    session.config_sent = false;
                }
                8 => {
                    session.pps = nalu.to_vec();
                    session.config_sent = false;
                }
                _ => {}
            }
        }
        let keyframe = nalus.iter().any(|n| !n.is_empty() && n[0] & 0x1f == 5);
        let cts = pts.zip(dts).map(|(p, d)| p.saturating_sub(d)).unwrap_or(0);
        if !session.config_sent && !session.sps.is_empty() && !session.pps.is_empty() {
            let avcc = avc_config(&session.sps, &session.pps);
            let mut d = vec![0x17, 0, 0, 0, 0];
            d.extend_from_slice(&avcc);
            stream.push_tag(9, session.ts, d);
            session.config_sent = true;
        }
        for nalu in nalus {
            if nalu.is_empty() || matches!(nalu[0] & 0x1f, 7 | 8) {
                continue;
            }
            let mut data = vec![if keyframe { 0x17 } else { 0x27 }, 1, 0, 0, 0];
            data[2..5].copy_from_slice(&cts.to_be_bytes()[1..]);
            data.extend_from_slice(&(nalu.len() as u32).to_be_bytes());
            data.extend_from_slice(nalu);
            stream.push_tag(9, session.ts, data);
        }
        session.ts = session.ts.wrapping_add(40);
    }
}
fn adts_frame(data: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    if data.len() < 7 || data[0] != 0xff || data[1] & 0xf0 != 0xf0 {
        return None;
    }
    let profile = (data[2] >> 6) & 0x03;
    let freq = (data[2] >> 2) & 0x0f;
    let channels = ((data[2] & 0x01) << 2) | (data[3] >> 6);
    let len =
        (((data[3] & 0x03) as usize) << 11) | ((data[4] as usize) << 3) | ((data[5] as usize) >> 5);
    if len < 7 || len > data.len() {
        return None;
    }
    let asc = vec![
        (profile << 3) | (freq >> 1),
        ((freq & 1) << 7) | (channels << 3),
    ];
    Some((data[7..len].to_vec(), asc))
}
fn avc_config(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut out = vec![1, sps[1], sps[2], sps[3], 0xff, 0xe1];
    out.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    out.extend_from_slice(sps);
    out.push(1);
    out.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    out.extend_from_slice(pps);
    out
}
fn annexb_nalus(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i..i + 3] == [0, 0, 1] {
            let start = i + 3;
            let end = data[start..]
                .windows(3)
                .position(|w| w == [0, 0, 1])
                .map(|p| start + p)
                .unwrap_or(data.len());
            if end > start {
                out.push(&data[start..end]);
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}
fn rtp_seq(packet: &[u8]) -> Option<u16> {
    (packet.len() >= 4).then(|| u16::from_be_bytes([packet[2], packet[3]]))
}
fn rtp_payload(packet: &[u8]) -> Option<&[u8]> {
    if packet.len() < 12 || (packet[0] >> 6) != 2 {
        return None;
    }
    let csrc = (packet[0] & 0x0f) as usize;
    let mut offset = 12 + csrc * 4;
    if packet[0] & 0x10 != 0 {
        if packet.len() < offset + 4 {
            return None;
        }
        let ext_len = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]) as usize;
        offset += 4 + ext_len * 4;
    }
    (offset <= packet.len()).then_some(&packet[offset..])
}

#[derive(Default)]
pub struct PsDemuxer {
    buffer: Vec<u8>,
}

impl PsDemuxer {
    pub fn push(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    #[allow(clippy::question_mark)]
    #[allow(clippy::type_complexity)]
    pub fn next_pes(&mut self) -> Option<(u8, Vec<u8>, Option<u32>, Option<u32>)> {
        loop {
            let start = self.buffer.windows(4).position(|w| w == [0, 0, 1, 0xba])?;
            if start > 0 {
                self.buffer.drain(..start);
            }
            if self.buffer.len() < 14 {
                return None;
            }
            self.buffer.drain(..14);
            loop {
                let Some(pos) = self.buffer.windows(3).position(|w| w == [0, 0, 1]) else {
                    return None;
                };
                if pos > 0 {
                    self.buffer.drain(..pos);
                }
                if self.buffer.len() < 6 {
                    return None;
                }
                let code = self.buffer[3];
                if code == 0xba {
                    break;
                }
                if code == 0xbb || code == 0xbe {
                    let len = u16::from_be_bytes([self.buffer[4], self.buffer[5]]) as usize;
                    let total = 6 + len;
                    if self.buffer.len() < total {
                        return None;
                    }
                    self.buffer.drain(..total);
                    continue;
                }
                let len = u16::from_be_bytes([self.buffer[4], self.buffer[5]]) as usize;
                if len == 0 {
                    self.buffer.drain(..6);
                    continue;
                }
                let total = 6 + len;
                if self.buffer.len() < total {
                    return None;
                }
                let payload_start = 9 + self.buffer[8] as usize;
                if payload_start >= total {
                    self.buffer.drain(..total);
                    continue;
                }
                let pts = if self.buffer[7] & 0x80 != 0 && self.buffer.len() >= 14 {
                    let b = &self.buffer[9..14];
                    Some(
                        ((((b[0] as u32 >> 1) & 7) << 30)
                            | ((b[1] as u32) << 22)
                            | ((b[2] as u32 >> 1) << 15)
                            | ((b[3] as u32) << 7)
                            | (b[4] as u32 >> 1))
                            / 90,
                    )
                } else {
                    None
                };
                let dts = if self.buffer[7] & 0x40 != 0 && self.buffer.len() >= 19 {
                    let b = &self.buffer[14..19];
                    Some(
                        ((((b[0] as u32 >> 1) & 7) << 30)
                            | ((b[1] as u32) << 22)
                            | ((b[2] as u32 >> 1) << 15)
                            | ((b[3] as u32) << 7)
                            | (b[4] as u32 >> 1))
                            / 90,
                    )
                } else {
                    None
                };
                let payload = self.buffer[payload_start..total].to_vec();
                self.buffer.drain(..total);
                return Some((code, payload, pts, dts));
            }
        }
    }
}
pub async fn run(socket: UdpSocket, state: Arc<GbState>) -> Result<()> {
    log::info!("GB28181 SIP listening on {}", socket.local_addr()?);
    let mut buf = [0u8; 8192];
    loop {
        let (n, peer) = socket.recv_from(&mut buf).await?;
        let text = String::from_utf8_lossy(&buf[..n]);
        for response in handle(&text, &state) {
            let _ = socket.send_to(response.as_bytes(), peer).await;
        }
    }
}

fn handle(text: &str, state: &GbState) -> Vec<String> {
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text, ""));
    let mut lines = head.lines();
    let Some(line) = lines.next() else {
        return Vec::new();
    };
    let method = line.split_whitespace().next().unwrap_or("");
    let headers: HashMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let from = headers.get("from").cloned().unwrap_or_default();
    let to = headers.get("to").cloned().unwrap_or_default();
    let via = headers.get("via").cloned().unwrap_or_default();
    let call_id = headers.get("call-id").cloned().unwrap_or_default();
    let cseq = headers.get("cseq").cloned().unwrap_or_default();
    let contact = headers.get("contact").cloned().unwrap_or_default();
    let expires = headers
        .get("expires")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(3600);
    let id = uri_user(&from).unwrap_or_else(|| "unknown".to_string());
    match method {
        "REGISTER" => {
            let nonce = state.fresh_nonce();
            if !state.password.is_empty()
                && !authorized(&headers, &state.domain, &state.password, &state.nonces)
            {
                return vec![challenge(
                    &state.domain,
                    &via,
                    &from,
                    &to,
                    &call_id,
                    &cseq,
                    &nonce,
                )];
            }
            if expires == 0 {
                state
                    .devices
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                return vec![response(
                    &state.domain,
                    &via,
                    &from,
                    &to,
                    &call_id,
                    &cseq,
                    None,
                    0,
                )];
            }
            state
                .devices
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .insert(
                    id.clone(),
                    Device {
                        id: id.clone(),
                        contact: contact.clone(),
                        expires_ms: now_ms() + expires * 1000,
                        last_seen_ms: now_ms(),
                    },
                );
            vec![response(
                &state.domain,
                &via,
                &from,
                &to,
                &call_id,
                &cseq,
                Some(&contact),
                expires,
            )]
        }
        "INVITE" => {
            let mode = if body.contains("s=Playback") || body.contains("S=Playback") { "Playback" } else { "Live" };
            *state.last_invite.write().unwrap_or_else(|e| e.into_inner()) = mode.to_string();
            if mode == "Playback" {
                if let Some(addr) = sdp_target(body) {
                    *state.playback_target.write().unwrap_or_else(|e| e.into_inner()) = Some(addr);
                    if let Ok(path) = std::env::var("M7S_GB_PLAYBACK_FILE") {
                        tokio::spawn(async move {
                            let Ok(sock) = tokio::net::UdpSocket::bind("0.0.0.0:0").await else { return };
                            let start = std::env::var("M7S_GB_PLAYBACK_START_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
                            let speed = std::env::var("M7S_GB_PLAYBACK_SPEED").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
                            let _ = crate::ps::play_flv_file_range(std::path::Path::new(&path), &sock, addr, start, speed).await;
                        });
                    }
                }
            }
            vec![invite_response(
                &state.domain,
                &via,
                &from,
                &to,
                &call_id,
                &cseq,
                state.media_port,
            )]
        }
        "ACK" => vec![response(
            &state.domain,
            &via,
            &from,
            &to,
            &call_id,
            &cseq,
            None,
            0,
        )],
        "BYE" => {
            state
                .sessions
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            vec![response(
                &state.domain,
                &via,
                &from,
                &to,
                &call_id,
                &cseq,
                None,
                0,
            )]
        }
        "MESSAGE" => {
            if let Some(d) = state
                .devices
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(&id)
            {
                d.last_seen_ms = now_ms();
            }
            let mut out = vec![response(
                &state.domain,
                &via,
                &from,
                &to,
                &call_id,
                &cseq,
                None,
                0,
            )];
            if body.contains("<CmdType>DeviceControl</CmdType>") {
                if let Some(cmd) = tag_value(body, "PTZCmd") {
                    *state.ptz.write().unwrap_or_else(|e| e.into_inner()) = Some(cmd.to_string());
                }
            }
            if body.contains("<CmdType>PlaybackControl</CmdType>") {
                let mut playback = state.playback.write().unwrap_or_else(|e| e.into_inner());
                if let Some(pause) = tag_value(body, "PAUSE") {
                    playback.paused = pause.eq_ignore_ascii_case("PAUSE");
                }
                if let Some(speed) = tag_value(body, "Speed").and_then(|v| v.parse::<f32>().ok()) {
                    playback.speed = speed;
                }
                if let Some(range) = tag_value(body, "Range") {
                    playback.range = range.to_string();
                }
            }
            if body.contains("<CmdType>Catalog</CmdType>") {
                out.push(catalog_message(state, &from, &to, body));
            }
            if body.contains("<CmdType>RecordInfo</CmdType>") {
                out.push(record_message(state, &from, body));
            }
            out
        }
        _ => Vec::new(),
    }
}
#[allow(clippy::too_many_arguments)]
fn sdp_target(body: &str) -> Option<std::net::SocketAddr> {
    let ip = body.lines().find_map(|l| l.trim().strip_prefix("c=IN IP4 "))?;
    let port: u16 = body
        .lines()
        .find_map(|l| l.trim().strip_prefix("m=video "))?
        .split_whitespace()
        .next()?
        .parse().ok()?;
    format!("{ip}:{port}").parse().ok()
}

fn invite_response(
    domain: &str,
    via: &str,
    from: &str,
    to: &str,
    call_id: &str,
    cseq: &str,
    port: u16,
) -> String {
    let to = if to.contains(";tag=") {
        to.to_string()
    } else {
        format!("{to};tag=m7s")
    };
    let sdp = format!(
        "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=m7s-rs\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=video {port} RTP/AVP 96\r\na=rtpmap:96 PS/90000\r\na=recvonly\r\ny=0100000001\r\n"
    );
    format!(
        "SIP/2.0 200 OK\r\nVia: {via}\r\nFrom: {from}\r\nTo: {to}\r\nCall-ID: {call_id}\r\nCSeq: {cseq}\r\nContact: <sip:server@{domain}>\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{sdp}",
        sdp.len()
    )
}
fn catalog_message(state: &GbState, from: &str, _to: &str, body: &str) -> String {
    let sn = tag_value(body, "SN").unwrap_or("1");
    let target = tag_value(body, "DeviceID")
        .map(str::to_string)
        .or_else(|| uri_user(from))
        .unwrap_or_default();
    let devices = state.devices.read().unwrap_or_else(|e| e.into_inner());
    let items: String = devices
        .values()
        .map(|d| {
            format!(
                "<Item><DeviceID>{}</DeviceID><Name>{}</Name><Status>ON</Status></Item>",
                d.id, d.id
            )
        })
        .collect();
    let xml = format!(
        "<?xml version=\"1.0\"?><Response><CmdType>Catalog</CmdType><SN>{sn}</SN><DeviceID>{target}</DeviceID><SumNum>{}</SumNum><DeviceList Num=\"{}\">{items}</DeviceList></Response>",
        devices.len(),
        devices.len()
    );
    format!(
        "MESSAGE sip:{target}@{} SIP/2.0\r\nVia: SIP/2.0/UDP {}\r\nFrom: <sip:server@{}>;tag=m7s\r\nTo: <sip:{target}@{}>\r\nCall-ID: catalog-{sn}\r\nCSeq: 1 MESSAGE\r\nContent-Type: Application/MANSCDP+xml\r\nContent-Length: {}\r\n\r\n{xml}",
        state.domain,
        state.domain,
        state.domain,
        state.domain,
        xml.len()
    )
}


fn record_message(state: &GbState, from: &str, body: &str) -> String {
    let sn = tag_value(body, "SN").unwrap_or("1");
    let target = tag_value(body, "DeviceID")
        .map(str::to_string)
        .or_else(|| uri_user(from))
        .unwrap_or_default();
    let start = tag_value(body, "StartTime").unwrap_or("");
    let end = tag_value(body, "EndTime").unwrap_or("");
    let mut items = String::new();
    let mut count = 0usize;
    if let Ok(dir) = std::env::var("M7S_GB_RECORD_DIR") {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                    continue;
                };
                if !matches!(ext.to_ascii_lowercase().as_str(), "mp4" | "flv" | "mkv") {
                    continue;
                }
                let time = record_time(name);
                if !start.is_empty() && time.as_str() < start {
                    continue;
                }
                if !end.is_empty() && time.as_str() > end {
                    continue;
                }
                count += 1;
                items.push_str(&format!(
                    "<Item><DeviceID>{target}</DeviceID><Name>{name}</Name><FilePath>{}</FilePath><StartTime>{time}</StartTime><EndTime>{time}</EndTime><Secrecy>0</Secrecy><Type>time</Type></Item>",
                    path.display()
                ));
            }
        }
    }
    let xml = format!(
        "<?xml version=\"1.0\"?><Response><CmdType>RecordInfo</CmdType><SN>{sn}</SN><DeviceID>{target}</DeviceID><SumNum>{count}</SumNum><RecordList Num=\"{count}\">{items}</RecordList></Response>"
    );
    format!(
        "MESSAGE sip:{target}@{} SIP/2.0\r\nVia: SIP/2.0/UDP {}\r\nFrom: <sip:server@{}>;tag=m7s\r\nTo: <sip:{target}@{}>\r\nCall-ID: record-{sn}\r\nCSeq: 1 MESSAGE\r\nContent-Type: Application/MANSCDP+xml\r\nContent-Length: {}\r\n\r\n{xml}",
        state.domain,
        state.domain,
        state.domain,
        state.domain,
        xml.len()
    )
}

fn record_time(name: &str) -> String {
    let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 14 {
        return format!(
            "{}-{}-{}T{}:{}:{}",
            &digits[0..4],
            &digits[4..6],
            &digits[6..8],
            &digits[8..10],
            &digits[10..12],
            &digits[12..14]
        );
    }
    "1970-01-01T00:00:00".to_string()
}
fn authorized(
    headers: &HashMap<String, String>,
    realm: &str,
    password: &str,
    nonces: &RwLock<HashMap<String, i64>>,
) -> bool {
    let Some(auth) = headers.get("authorization") else {
        return false;
    };
    let Some(username) = digest_param(auth, "username") else {
        return false;
    };
    let Some(uri) = digest_param(auth, "uri") else {
        return false;
    };
    let Some(response) = digest_param(auth, "response") else {
        return false;
    };
    let Some(nonce) = digest_param(auth, "nonce") else {
        return false;
    };
    {
        let values = nonces.read().unwrap_or_else(|e| e.into_inner());
        let Some(expires) = values.get(nonce) else {
            return false;
        };
        if *expires < now_ms() {
            return false;
        }
    }
    let ha1 = md5_hex(&format!("{username}:{realm}:{password}"));
    let ha2 = md5_hex(&format!("REGISTER:{uri}"));
    let expected = match digest_param(auth, "qop") {
        Some(qop) => {
            let nc = digest_param(auth, "nc").unwrap_or("00000001");
            let cnonce = digest_param(auth, "cnonce").unwrap_or("");
            md5_hex(&format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}"))
        }
        None => md5_hex(&format!("{ha1}:{nonce}:{ha2}")),
    };
    if response.eq_ignore_ascii_case(&expected) {
        nonces
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(nonce);
        true
    } else {
        false
    }
}

#[allow(clippy::too_many_arguments)]
fn challenge(
    realm: &str,
    via: &str,
    from: &str,
    to: &str,
    call_id: &str,
    cseq: &str,
    nonce: &str,
) -> String {
    let to = if to.contains(";tag=") {
        to.to_string()
    } else {
        format!("{to};tag=m7s")
    };
    format!(
        "SIP/2.0 401 Unauthorized\r\nVia: {via}\r\nFrom: {from}\r\nTo: {to}\r\nCall-ID: {call_id}\r\nCSeq: {cseq}\r\nWWW-Authenticate: Digest realm=\"{realm}\", nonce=\"{nonce}\", qop=\"auth\", algorithm=MD5\r\nContent-Length: 0\r\n\r\n"
    )
}

#[allow(clippy::too_many_arguments)]
fn response(
    domain: &str,
    via: &str,
    from: &str,
    to: &str,
    call_id: &str,
    cseq: &str,
    contact: Option<&str>,
    expires: i64,
) -> String {
    let to = if to.contains(";tag=") {
        to.to_string()
    } else {
        format!("{to};tag=m7s")
    };
    let mut out = format!(
        "SIP/2.0 200 OK\r\nVia: {via}\r\nFrom: {from}\r\nTo: {to}\r\nCall-ID: {call_id}\r\nCSeq: {cseq}\r\n"
    );
    if let Some(contact) = contact {
        out.push_str(&format!("Contact: {contact}\r\nExpires: {expires}\r\n"));
    }
    out.push_str(&format!(
        "Server: m7s-rs/{domain}\r\nContent-Length: 0\r\n\r\n"
    ));
    out
}


#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, timeout};

    struct Null;
    impl StreamApi for Null {
        fn stream(&self, _app: &str, _name: &str) -> Option<Arc<dyn m7s_sdk::TagStream>> {
            None
        }
        fn get(&self, _app: &str, _name: &str) -> Option<Arc<dyn m7s_sdk::TagStream>> {
            None
        }
        fn list(&self) -> Vec<m7s_sdk::StreamInfo> {
            Vec::new()
        }
        fn count(&self) -> usize {
            0
        }
    }
    fn null_streams() -> Arc<dyn StreamApi> {
        Arc::new(Null)
    }

    #[test]
    fn md5_vectors() {
        assert_eq!(md5_hex(""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex("abc"), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn parses_user() {
        assert_eq!(
            uri_user("<sip:34020000001320000001@3402000000>"),
            Some("34020000001320000001".to_string())
        );
    }

    #[test]
    fn registers_device() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let state = GbState::new("3402000000", "", 30000, null_streams());
            tokio::spawn(run(listener, state.clone()));
            let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let request = "REGISTER sip:3402000000 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1\r\nFrom: <sip:34020000001320000001@3402000000>;tag=x\r\nTo: <sip:34020000001320000001@3402000000>\r\nCall-ID: 1\r\nCSeq: 1 REGISTER\r\nContact: <sip:34020000001320000001@127.0.0.1>\r\nExpires: 3600\r\nContent-Length: 0\r\n\r\n";
            client.send_to(request.as_bytes(), addr).await.unwrap();
            let mut buf = [0u8; 2048];
            let (n, _) = timeout(Duration::from_secs(2), client.recv_from(&mut buf))
                .await
                .unwrap()
                .unwrap();
            assert!(String::from_utf8_lossy(&buf[..n]).starts_with("SIP/2.0 200 OK"));
            assert_eq!(state.device_count(), 1);
        });
    }
    #[test]
    fn register_without_digest_is_401() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let state = GbState::new("3402000000", "12345", 30000, null_streams());
            tokio::spawn(run(listener, state.clone()));
            let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let request = "REGISTER sip:3402000000 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1\r\nFrom: <sip:34020000001320000001@3402000000>;tag=x\r\nTo: <sip:34020000001320000001@3402000000>\r\nCall-ID: 1\r\nCSeq: 1 REGISTER\r\nContact: <sip:34020000001320000001@127.0.0.1>\r\nExpires: 3600\r\nContent-Length: 0\r\n\r\n";
            client.send_to(request.as_bytes(), addr).await.unwrap();
            let mut buf = [0u8; 2048];
            let (n, _) = timeout(Duration::from_secs(2), client.recv_from(&mut buf)).await.unwrap().unwrap();
            assert!(String::from_utf8_lossy(&buf[..n]).starts_with("SIP/2.0 401 Unauthorized"));
            assert_eq!(state.device_count(), 0);
        });
    }
    #[test]
    fn catalog_lists_registered_device() {
        let state = GbState::new(
            "3402000000",
            "",
            30000,
            null_streams(),
        );
        let register = "REGISTER sip:3402000000 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1\r\nFrom: <sip:34020000001320000001@3402000000>;tag=x\r\nTo: <sip:34020000001320000001@3402000000>\r\nCall-ID: 1\r\nCSeq: 1 REGISTER\r\nContact: <sip:34020000001320000001@127.0.0.1>\r\nExpires: 3600\r\nContent-Length: 0\r\n\r\n";
        assert_eq!(handle(register, &state).len(), 1);
        let query = "MESSAGE sip:3402000000 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1\r\nFrom: <sip:34020000001320000001@3402000000>;tag=x\r\nTo: <sip:3402000000@3402000000>\r\nCall-ID: 2\r\nCSeq: 1 MESSAGE\r\nContent-Type: Application/MANSCDP+xml\r\nContent-Length: 74\r\n\r\n<Query><CmdType>Catalog</CmdType><SN>1</SN><DeviceID>34020000001320000001</DeviceID></Query>";
        let responses = handle(query, &state);
        assert_eq!(responses.len(), 2);
        assert!(responses[1].contains("<CmdType>Catalog</CmdType>"));
        assert!(responses[1].contains("34020000001320000001"));
    }
    #[test]
    fn invite_playback_records_mode() {
        let state = GbState::new("3402000000", "", 30000, null_streams());
        let request = "INVITE sip:34020000001320000001@3402000000 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1\r\nFrom: <sip:3402000000@3402000000>;tag=x\r\nTo: <sip:34020000001320000001@3402000000>\r\nCall-ID: 9\r\nCSeq: 1 INVITE\r\nContent-Type: application/sdp\r\nContent-Length: 0\r\n\r\nv=0\r\ns=Playback\r\nt=0 0\r\n";
        let responses = handle(request, &state);
        assert!(responses[0].starts_with("SIP/2.0 200 OK"));
        assert_eq!(state.last_invite(), "Playback");
    }

    #[test]
    fn invite_playback_records_target() {
        let state = GbState::new("3402000000", "", 30000, null_streams());
        let request = "INVITE sip:34020000001320000001@3402000000 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1\r\nFrom: <sip:3402000000@3402000000>;tag=x\r\nTo: <sip:34020000001320000001@3402000000>\r\nCall-ID: 10\r\nCSeq: 1 INVITE\r\nContent-Type: application/sdp\r\nContent-Length: 0\r\n\r\nv=0\r\ns=Playback\r\nc=IN IP4 127.0.0.1\r\nm=video 40000 RTP/AVP 96\r\n";
        let _ = handle(request, &state);
        assert_eq!(
            state.playback_target(),
            Some("127.0.0.1:40000".parse().unwrap())
        );
    }

    #[test]
    fn ptz_control_records_command() {
        let state = GbState::new("3402000000", "", 30000, null_streams());
        let msg = "MESSAGE sip:3402000000 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1\r\nFrom: <sip:34020000001320000001@3402000000>;tag=x\r\nTo: <sip:3402000000@3402000000>\r\nCall-ID: 3\r\nCSeq: 1 MESSAGE\r\nContent-Type: Application/MANSCDP+xml\r\nContent-Length: 0\r\n\r\n<Control><CmdType>DeviceControl</CmdType><SN>1</SN><DeviceID>34020000001320000001</DeviceID><PTZCmd>ABCD1234</PTZCmd></Control>";
        let responses = handle(msg, &state);
        assert_eq!(responses.len(), 1);
        assert!(responses[0].starts_with("SIP/2.0 200 OK"));
        assert_eq!(state.last_ptz().as_deref(), Some("ABCD1234"));
    }

    #[test]
    fn playback_control_updates_state() {
        let state = GbState::new("3402000000", "", 30000, null_streams());
        let msg = "MESSAGE sip:3402000000 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1\r\nFrom: <sip:34020000001320000001@3402000000>;tag=x\r\nTo: <sip:3402000000@3402000000>\r\nCall-ID: pc\r\nCSeq: 1 MESSAGE\r\nContent-Type: Application/MANSCDP+xml\r\nContent-Length: 0\r\n\r\n<Control><CmdType>PlaybackControl</CmdType><SN>1</SN><DeviceID>34020000001320000001</DeviceID><PAUSE>PAUSE</PAUSE><Speed>2.0</Speed><Range>2026-11-21T10:00:00Z/2026-11-21T10:05:00Z</Range></Control>";
        let responses = handle(msg, &state);
        assert!(responses[0].starts_with("SIP/2.0 200 OK"));
        let playback = state.playback();
        assert!(playback.paused);
        assert_eq!(playback.speed, 2.0);
        assert_eq!(playback.range, "2026-11-21T10:00:00Z/2026-11-21T10:05:00Z");
    }

    #[test]
    fn record_info_returns_empty_list() {
        let state = GbState::new("3402000000", "", 30000, null_streams());
        let msg = "MESSAGE sip:3402000000 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1\r\nFrom: <sip:34020000001320000001@3402000000>;tag=x\r\nTo: <sip:3402000000@3402000000>\r\nCall-ID: ri\r\nCSeq: 1 MESSAGE\r\nContent-Type: Application/MANSCDP+xml\r\nContent-Length: 0\r\n\r\n<Query><CmdType>RecordInfo</CmdType><SN>7</SN><DeviceID>34020000001320000001</DeviceID></Query>";
        let responses = handle(msg, &state);
        assert_eq!(responses.len(), 2);
        assert!(responses[1].contains("<CmdType>RecordInfo</CmdType>"));
        assert!(responses[1].contains("<RecordList Num=\"0\">"));
    }

    #[test]
    fn register_with_qop_digest() {
        let state = GbState::new(
            "3402000000",
            "12345",
            30000,
            null_streams(),
        );
        let nonce = "abc123";
        state
            .nonces
            .write()
            .unwrap()
            .insert(nonce.to_string(), now_ms() + 60_000);
        let user = "34020000001320000001";
        let uri = "sip:3402000000";
        let nc = "00000001";
        let cnonce = "clientnonce";
        let ha1 = md5_hex(&format!("{user}:3402000000:12345"));
        let ha2 = md5_hex(&format!("REGISTER:{uri}"));
        let resp = md5_hex(&format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}"));
        let request = format!(
            "REGISTER sip:3402000000 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1\r\nFrom: <sip:{user}@3402000000>;tag=x\r\nTo: <sip:{user}@3402000000>\r\nCall-ID: 1\r\nCSeq: 1 REGISTER\r\nContact: <sip:{user}@127.0.0.1>\r\nAuthorization: Digest username=\"{user}\", realm=\"3402000000\", nonce=\"{nonce}\", uri=\"{uri}\", response=\"{resp}\", qop=auth, nc={nc}, cnonce=\"{cnonce}\"\r\nExpires: 3600\r\nContent-Length: 0\r\n\r\n"
        );
        let responses = handle(&request, &state);
        assert!(responses[0].starts_with("SIP/2.0 200 OK"));
        assert_eq!(state.device_count(), 1);
    }
    #[test]
    fn invite_returns_sdp() {
        let state = GbState::new(
            "3402000000",
            "",
            30000,
            null_streams(),
        );
        let request = "INVITE sip:34020000001320000001@3402000000 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1\r\nFrom: <sip:3402000000@3402000000>;tag=x\r\nTo: <sip:34020000001320000001@3402000000>\r\nCall-ID: 3\r\nCSeq: 1 INVITE\r\nContent-Type: application/sdp\r\nContent-Length: 0\r\n\r\n";
        let responses = handle(request, &state);
        assert!(responses[0].starts_with("SIP/2.0 200 OK"));
        assert!(responses[0].contains("m=video 30000 RTP/AVP 96"));
        assert!(responses[0].contains("PS/90000"));
    }
    #[test]
    fn parses_rtp_ps_payload() {
        let mut packet = vec![0x80, 96, 0, 1, 0, 0, 0, 1, 0x12, 0x34, 0x56, 0x78];
        packet.extend_from_slice(&[0, 0, 1, 0xba, 1, 2, 3]);
        let payload = rtp_payload(&packet).unwrap();
        assert!(payload.windows(4).any(|w| w == [0, 0, 1, 0xba]));
    }
    #[test]
    fn demuxes_ps_video_pes() {
        let mut ps = PsDemuxer::default();
        ps.push(&[0, 0, 1, 0xba, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        ps.push(&[0, 0, 1, 0xe0, 0, 7, 0x80, 0, 0, 0x65, 1, 2, 3]);
        let (code, payload, _, _) = ps.next_pes().unwrap();
        assert_eq!(code, 0xe0);
        assert_eq!(payload, vec![0x65, 1, 2, 3]);
    }
    #[test]
    fn parses_adts() {
        let data = [0xff, 0xf1, 0x50, 0x80, 0x01, 0x20, 0xfc, 0x11, 0x22];
        let (raw, asc) = adts_frame(&data).unwrap();
        assert_eq!(raw.len(), 2);
        assert_eq!(asc[1] >> 3, 2);
    }
    #[test]
    fn unregister_expires_zero() {
        let state = GbState::new(
            "3402000000",
            "",
            30000,
            null_streams(),
        );
        let base = "REGISTER sip:3402000000 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1\r\nFrom: <sip:34020000001320000001@3402000000>;tag=x\r\nTo: <sip:34020000001320000001@3402000000>\r\nCall-ID: 1\r\nCSeq: 1 REGISTER\r\nContact: <sip:34020000001320000001@127.0.0.1>\r\nExpires: 3600\r\nContent-Length: 0\r\n\r\n";
        assert_eq!(handle(base, &state).len(), 1);
        assert_eq!(state.device_count(), 1);
        let unreg = base.replace("Expires: 3600", "Expires: 0");
        assert_eq!(handle(&unreg, &state).len(), 1);
        assert_eq!(state.device_count(), 0);
    }
    #[test]
    fn expires_devices() {
        let state = GbState::new(
            "3402000000",
            "",
            30000,
            null_streams(),
        );
        state.devices.write().unwrap().insert(
            "1".to_string(),
            Device {
                id: "1".to_string(),
                contact: String::new(),
                expires_ms: now_ms() - 1,
                last_seen_ms: now_ms() - 1,
            },
        );
        expire_devices(&state);
        assert_eq!(state.device_count(), 0);
    }
    #[test]
    fn record_info_lists_files() {
        let dir = std::env::temp_dir().join(format!("m7s-gb-rec-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("20261122120000.mp4");
        std::fs::write(&file, b"mp4").unwrap();
        unsafe { std::env::set_var("M7S_GB_RECORD_DIR", &dir) };
        let state = GbState::new("3402000000", "", 30000, null_streams());
        let body = "<CmdType>RecordInfo</CmdType><SN>7</SN><DeviceID>34020000001320000001</DeviceID><StartTime>2026-11-01T00:00:00</StartTime><EndTime>2026-12-01T00:00:00</EndTime>";
        let msg = record_message(&state, "<sip:34020000001320000001@3402000000>", body);
        assert!(msg.contains("<SumNum>1</SumNum>"), "{msg}");
        assert!(msg.contains("20261122120000.mp4"), "{msg}");
        assert!(msg.contains("2026-11-22T12:00:00"), "{msg}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

pub fn tag_value<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(&xml[start..end])
}

pub fn digest_param<'a>(auth: &'a str, key: &str) -> Option<&'a str> {
    let auth = auth.strip_prefix("Digest ").unwrap_or(auth);
    auth.split(',')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.trim().trim_matches('"'))
}

pub fn md5_hex(input: &str) -> String {
    use md5::Digest;
    let mut hasher = md5::Md5::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn uri_user(value: &str) -> Option<String> {
    let start = value.find("sip:")? + 4;
    let rest = &value[start..];
    let end = rest.find('@').or_else(|| rest.find(' ')).unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

impl GbState {
    pub fn with_context(
        domain: &str,
        password: &str,
        media_port: u16,
        ctx: m7s_sdk::EngineContext,
    ) -> Arc<Self> {
        let streams = ctx.stream_api().expect("GB28181 requires StreamApi");
        let state = Self::new(domain, password, media_port, streams);
        Arc::new(Self {
            domain: state.domain.clone(),
            password: state.password.clone(),
            media_port: state.media_port,
            devices: RwLock::new(HashMap::new()),
            nonces: RwLock::new(HashMap::new()),
            media: RwLock::new(MediaStats::default()),
            streams: state.streams.clone(),
            sessions: RwLock::new(HashMap::new()),
            ptz: RwLock::new(None),
            playback: RwLock::new(PlaybackState::default()),
            last_invite: RwLock::new(String::new()),
            playback_target: RwLock::new(None),
            context: Some(ctx),
        })
    }

    pub fn context(&self) -> Option<&m7s_sdk::EngineContext> {
        self.context.as_ref()
    }
}
