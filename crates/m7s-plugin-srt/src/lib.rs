#![allow(clippy::collapsible_if)]
//! SRT listener + MPEG-TS demux (H.264/AAC -> FLV tags).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use sapflow_srt::crypto::{CryptoConfig, KeySize};
use sapflow_srt::{Caller, CallerConfig, Config, Event, Listener};
use m7s_plugin_hls::Segmenter;
use tokio::net::UdpSocket;

use m7s_sdk::{StreamApi, TagSink, TagStream};


const TS: usize = 188;

pub async fn run_with_context(bind: &str, ctx: m7s_sdk::EngineContext) -> Result<()> {
    let streams = ctx.stream_api().ok_or_else(|| anyhow::anyhow!("EngineContext missing streams"))?;
    run(bind, streams).await
}

pub async fn run(bind: &str, streams: Arc<dyn StreamApi>) -> Result<()> {
    let socket = UdpSocket::bind(bind).await?;
    log::info!("SRT listening on {}", socket.local_addr()?);
    let mut config = Config::default();
    if let Ok(pass) = std::env::var("M7S_SRT_PASSPHRASE") {
        config.crypto = Some(CryptoConfig::new(pass.into_bytes(), KeySize::Aes128)?);
        config.require_encryption = true;
        log::info!("SRT encryption enabled (AES-128)");
    }
    let mut listener = Listener::new(config, Instant::now());
    let mut buf = [0u8; 2048];
    let mut sessions: HashMap<u32, TsDemux> = HashMap::new();
    let mut guards: HashMap<u32, m7s_sdk::connections::ConnectionGuard> = HashMap::new();

    loop {
        if !m7s_sdk::plugins::enabled("srt") {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        }
        let now = Instant::now();
        let wait = listener
            .poll_timeout()
            .map_or(Duration::from_millis(100), |at| {
                at.saturating_duration_since(now)
            })
            .max(Duration::from_millis(1));
        tokio::select! {
            recv = socket.recv_from(&mut buf) => {
                if let Ok((n, peer)) = recv {
                    if let Some(d) = buf.get(..n) {
                        listener.handle_datagram(Instant::now(), peer, d);
                    }
                }
            }
            _ = tokio::time::sleep(wait) => {}
        }
        listener.handle_timeout(Instant::now());

        while let Some(event) = listener.poll_event() {
            match event {
                Event::Connect {
                    id,
                    peer,
                    stream_id,
                } => {
                    let sid = String::from_utf8_lossy(&stream_id).to_string();
                    log::info!("SRT connect {peer} stream_id={sid}");
                    let Some(guard) = m7s_sdk::connections::try_acquire() else {
                        log::warn!("SRT rejected {sid}: connection limit");
                        continue;
                    };
                    let (app, name) = sid.split_once('/').unwrap_or(("live", "test"));
                    let Some(stream) = streams.stream(app, name) else {
                        log::warn!("SRT rejected {sid}: stream seat limit");
                        continue;
                    };
                    sessions.insert(id.0, TsDemux::new(stream));
                    guards.insert(id.0, guard);
                    listener.accept(id, Instant::now());
                }
                Event::Data { id, payload, .. } => {
                    if let Some(d) = sessions.get_mut(&id.0) {
                        d.push(&payload);
                    }
                }
                Event::Closed { id, reason, .. } => {
                    if let Some(mut d) = sessions.remove(&id.0) {
                        d.flush();
                    }
                    log::info!("SRT closed {id:?}: {reason:?}");
                }
            }
        }
        while let Some((to, datagram)) = listener.poll_transmit() {
            let _ = socket.send_to(&datagram, to).await;
        }
    }
}

pub struct TsDemux {
    sink: Arc<dyn TagSink>,
    buf: Vec<u8>,
    pmt_pid: u16,
    video_pid: u16,
    audio_pid: u16,
    video: Vec<u8>,
    audio: Vec<u8>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    config_sent: bool,
    vbase: Option<u32>,
    abase: Option<u32>,
}

impl TsDemux {
    pub fn new(sink: Arc<dyn TagSink>) -> Self {
        Self {
            sink,
            buf: Vec::new(),
            pmt_pid: 0,
            video_pid: 0,
            audio_pid: 0,
            video: Vec::new(),
            audio: Vec::new(),
            sps: None,
            pps: None,
            config_sent: false,
            vbase: None,
            abase: None,
        }
    }

    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
        while self.buf.len() >= TS {
            let pkt: Vec<u8> = self.buf.drain(..TS).collect();
            self.packet(&pkt);
        }
    }

    fn packet(&mut self, p: &[u8]) {
        if p[0] != 0x47 {
            return;
        }
        let pid = (((p[1] & 0x1f) as u16) << 8) | p[2] as u16;
        let pusi = p[1] & 0x40 != 0;
        let afc = (p[3] >> 4) & 3;
        let mut off = 4usize;
        if afc & 2 != 0 {
            if off >= TS {
                return;
            }
            off += 1 + p[off] as usize;
        }
        if afc & 1 == 0 || off >= TS {
            return;
        }
        let payload = &p[off..];
        if pid == 0 {
            self.pat(payload, pusi);
        } else if pid == self.pmt_pid {
            self.pmt(payload, pusi);
        } else if pid == self.video_pid {
            if pusi {
                self.flush_video();
                self.video.clear();
            }
            self.video.extend_from_slice(payload);
        } else if pid == self.audio_pid {
            if pusi {
                self.flush_audio();
                self.audio.clear();
            }
            self.audio.extend_from_slice(payload);
        }
    }

    fn section<'a>(&self, payload: &'a [u8], pusi: bool) -> Option<&'a [u8]> {
        if pusi {
            let ptr = *payload.first()? as usize;
            payload.get(1 + ptr..)
        } else {
            Some(payload)
        }
    }

    fn pat(&mut self, payload: &[u8], pusi: bool) {
        let Some(s) = self.section(payload, pusi) else {
            return;
        };
        if s.len() < 12 || s[0] != 0 {
            return;
        }
        let end = 3 + ((((s[1] & 0x0f) as usize) << 8) | s[2] as usize);
        let mut i = 8;
        while i + 4 <= end.min(s.len()) {
            let program = u16::from_be_bytes([s[i], s[i + 1]]);
            let pid = (((s[i + 2] & 0x1f) as u16) << 8) | s[i + 3] as u16;
            if program != 0 {
                self.pmt_pid = pid;
                break;
            }
            i += 4;
        }
    }

    fn pmt(&mut self, payload: &[u8], pusi: bool) {
        let Some(s) = self.section(payload, pusi) else {
            return;
        };
        if s.len() < 12 || s[0] != 2 {
            return;
        }
        let end = 3 + ((((s[1] & 0x0f) as usize) << 8) | s[2] as usize);
        let pil = (((s[10] & 0x0f) as usize) << 8) | s[11] as usize;
        let mut i = 12 + pil;
        while i + 5 <= end.min(s.len()) {
            let st = s[i];
            let pid = (((s[i + 1] & 0x1f) as u16) << 8) | s[i + 2] as u16;
            let es = (((s[i + 3] & 0x0f) as usize) << 8) | s[i + 4] as usize;
            match st {
                0x1b => self.video_pid = pid,
                0x0f => self.audio_pid = pid,
                _ => {}
            }
            i += 5 + es;
        }
    }

    fn flush(&mut self) {
        self.flush_video();
        self.flush_audio();
    }

    fn pes_payload(pes: &[u8]) -> Option<(u32, &[u8])> {
        if pes.len() < 9 || pes[0] != 0 || pes[1] != 0 || pes[2] != 1 {
            return None;
        }
        let flags = pes[7];
        let hlen = pes[8] as usize;
        let start = 9 + hlen;
        if start >= pes.len() {
            return None;
        }
        let mut pts = 0u32;
        if flags & 0x80 != 0 && pes.len() >= 14 {
            pts = (((pes[9] >> 1) & 0x07) as u32) << 30
                | (pes[10] as u32) << 22
                | ((pes[11] >> 1) as u32) << 15
                | (pes[12] as u32) << 7
                | (pes[13] >> 1) as u32;
        }
        Some((pts, &pes[start..]))
    }

    fn flush_video(&mut self) {
        if self.video.is_empty() {
            return;
        }
        let pes = std::mem::take(&mut self.video);
        let Some((pts, data)) = Self::pes_payload(&pes) else {
            return;
        };
        let mut nalus = Vec::new();
        let mut i = 0;
        while i + 3 < data.len() {
            if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
                let start = i + 3;
                let mut end = data.len();
                let mut j = start;
                while j + 3 < data.len() {
                    if data[j] == 0 && data[j + 1] == 0 && data[j + 2] == 1 {
                        end = j;
                        break;
                    }
                    j += 1;
                }
                if start < end {
                    nalus.push(&data[start..end]);
                }
                i = end;
            } else {
                i += 1;
            }
        }
        let mut key = false;
        for n in &nalus {
            match n[0] & 0x1f {
                7 => self.sps = Some(n.to_vec()),
                8 => self.pps = Some(n.to_vec()),
                5 => key = true,
                _ => {}
            }
        }
        if !self.config_sent {
            if let (Some(s), Some(p)) = (&self.sps, &self.pps) {
                self.sink.push_tag(9, 0, avc_config(s, p));
                self.config_sent = true;
            }
        }
        if !self.config_sent || nalus.is_empty() {
            return;
        }
        let mut avcc = Vec::new();
        for n in &nalus {
            avcc.extend_from_slice(&(n.len() as u32).to_be_bytes());
            avcc.extend_from_slice(n);
        }
        let base = *self.vbase.get_or_insert(pts);
        let ts = pts.wrapping_sub(base) / 90;
        let mut tag = vec![if key { 0x17 } else { 0x27 }, 1, 0, 0, 0];
        tag.extend_from_slice(&avcc);
        self.sink.push_tag(9, ts, tag);
    }

    fn flush_audio(&mut self) {
        if self.audio.is_empty() {
            return;
        }
        let pes = std::mem::take(&mut self.audio);
        let Some((pts, data)) = Self::pes_payload(&pes) else {
            return;
        };
        let mut i = 0;
        while i + 7 <= data.len() {
            if data[i] != 0xff || data[i + 1] & 0xf0 != 0xf0 {
                i += 1;
                continue;
            }
            let len = (((data[i + 3] & 0x03) as usize) << 11)
                | ((data[i + 4] as usize) << 3)
                | ((data[i + 5] as usize) >> 5);
            if len < 7 || i + len > data.len() {
                break;
            }
            let base = *self.abase.get_or_insert(pts);
            let ts = pts.wrapping_sub(base) / 90;
            let mut tag = vec![0xaf, 1];
            tag.extend_from_slice(&data[i + 7..i + len]);
            self.sink.push_tag(8, ts, tag);
            i += len;
        }
    }
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
/// SRT egress: mux a stream into MPEG-TS and push it to an SRT listener.
pub async fn push(stream: Arc<dyn TagStream>, target: &str, stream_id: &str) -> Result<()> {
    let peer: std::net::SocketAddr = target.strip_prefix("srt://").unwrap_or(target).parse()?;
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let cfg = CallerConfig { stream_id: stream_id.to_string(), ..CallerConfig::default() };
    let mut caller = Caller::new(cfg, peer, Instant::now())?;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);

    let mut reader = stream.tag_reader();
    tokio::spawn(async move {
        let mut segmenter = Segmenter::new(1);
        loop {
            if let Some(tag) = reader.try_next() {
                let mut outputs = Vec::new();
                segmenter.push(&tag, &mut outputs);
                for output in outputs {
                    if tx.send(output.data).await.is_err() {
                        return;
                    }
                }
            } else {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    });

    let mut buf = [0u8; 2048];
    loop {
        let now = Instant::now();
        while let Some((to, data)) = caller.poll_transmit(now) {
            let _ = socket.send_to(&data, to).await;
        }
        if caller.is_connected() {
            while caller.stats().send.buffered_packets < 8 {
                match rx.try_recv() {
                    Ok(data) => {
                        for chunk in data.chunks(1316) {
                            if caller.enqueue(Instant::now(), chunk.to_vec()).is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        }
        if let Ok(Ok((n, from))) =
            tokio::time::timeout(Duration::from_millis(10), socket.recv_from(&mut buf)).await
        {
            caller.handle_datagram(Instant::now(), from, &buf[..n]);
        }
        caller.handle_timeout(Instant::now());
        while caller.poll_event().is_some() {}
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

#[cfg(test)]
mod demux_tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Sink {
        tags: Mutex<Vec<(u8, u32, Vec<u8>)>>,
    }

    impl TagSink for Sink {
        fn push_tag(&self, tag_type: u8, timestamp: u32, data: Vec<u8>) {
            self.tags.lock().unwrap().push((tag_type, timestamp, data));
        }
        fn leave(&self) {}
    }

    #[test]
    fn demuxes_ts_video_tag() {
        let mut mux = m7s_codec::ts::TsMuxer::new();
        let mut ts = Vec::new();
        mux.write_tables(&mut ts);
        let annexb = vec![
            0, 0, 0, 1, 0x67, 0x42, 0, 0x1e, 0xE1, 0x01, 0x40,
            0, 0, 0, 1, 0x68, 0xCE, 0x3C, 0x80,
            0, 0, 0, 1, 0x65, 0x88, 0x84, 0x00,
        ];
        mux.write_video(&mut ts, &annexb, 0, 0);
        let sink = Arc::new(Sink::default());
        let mut demux = TsDemux::new(sink.clone());
        demux.push(&ts);
        demux.flush();
        let tags = sink.tags.lock().unwrap();
        assert!(!tags.is_empty());
        assert!(tags.iter().any(|t| t.0 == 9));
    }
}

