#![allow(clippy::manual_is_multiple_of, clippy::collapsible_if)]
//! WebRTC plugins (WHEP/WHIP).
pub mod whep;
pub mod whip;

use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;

pub fn ice_servers(urls: &str, username: &str, credential: &str) -> Vec<RTCIceServer> {
    urls.split(',')
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(|u| RTCIceServer {
            urls: vec![u.to_string()],
            username: username.to_string(),
            credential: credential.to_string(),
            ..Default::default()
        })
        .collect()
}

pub fn rtc_config() -> RTCConfiguration {
    let urls = std::env::var("M7S_TURN_URL").unwrap_or_default();
    let user = std::env::var("M7S_TURN_USER").unwrap_or_default();
    let pass = std::env::var("M7S_TURN_PASS").unwrap_or_default();
    RTCConfiguration {
        ice_servers: ice_servers(&urls, &user, &pass),
        ..Default::default()
    }
}

#[cfg(test)]
mod ice_tests {
    use super::*;
    #[test]
    fn parses_turn() {
        let s = ice_servers("turn:1.2.3.4:3478?transport=udp,turns:5.6.7.8:5349", "u", "p");
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].urls[0], "turn:1.2.3.4:3478?transport=udp");
        assert_eq!(s[0].username, "u");
    }
}

/// Returns the preferred RID from `a=simulcast:send h;m;l`.
pub fn simulcast_rid(sdp: &str) -> Option<String> {
    let line = sdp.lines().find(|l| l.contains("a=simulcast:send "))?;
    let list = line.split("a=simulcast:send ").nth(1)?.trim();
    list.split(';')
        .map(|s| s.trim().trim_start_matches('~'))
        .find(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod simulcast_tests {
    use super::*;
    #[test]
    fn picks_first_rid() {
        assert_eq!(simulcast_rid("a=simulcast:send h;m;l").as_deref(), Some("h"));
        assert_eq!(simulcast_rid("no simulcast"), None);
    }
}

static RID_OVERRIDE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>> = std::sync::OnceLock::new();

fn rid_map() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
    RID_OVERRIDE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

pub fn twcc_rid(total: usize, lost: usize) -> &'static str {
    if total == 0 { "m" } else if lost * 20 > total { "l" } else if lost == 0 { "h" } else { "m" }
}

pub fn auto_rid(nacks: u64, rtt_ms: f64, current: &str) -> String {
    if nacks > 10 || rtt_ms > 300.0 { "l".to_string() } else if nacks == 0 && rtt_ms < 150.0 { "h".to_string() } else { current.to_string() }
}

pub fn set_rid(stream: &str, rid: &str) {
    rid_map().lock().unwrap_or_else(|e| e.into_inner()).insert(stream.to_string(), rid.to_string());
}

pub fn rid_for(stream: &str, default: Option<&String>) -> Option<String> {
    rid_map()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(stream)
        .cloned()
        .or_else(|| default.cloned())
}

#[cfg(test)]
mod rid_override_tests {
    use super::*;
    #[test]
    fn override_map() {
        set_rid("live/t", "l");
        let d = "h".to_string();
        assert_eq!(rid_for("live/t", Some(&d)).as_deref(), Some("l"));
        assert_eq!(rid_for("live/other", Some(&d)).as_deref(), Some("h"));
        assert_eq!(auto_rid(20, 20.0, "h"), "l");
        assert_eq!(auto_rid(0, 20.0, "l"), "h");
        assert_eq!(auto_rid(5, 200.0, "m"), "m");
        assert_eq!(auto_rid(0, 400.0, "h"), "l");
        assert_eq!(twcc_rid(100, 0), "h");
        assert_eq!(twcc_rid(100, 10), "l");
        assert_eq!(twcc_rid(100, 2), "m");
    }
}

/// Parse a REMB packet (`RTCP PSFB`, FMT=15) and return the exponent/mantissa
/// bitrate in bps.
pub fn parse_remb(packet: &[u8]) -> Option<u64> {
    if packet.len() < 16 || packet[1] != 206 || (packet[0] & 0x1f) != 15 {
        return None;
    }
    if &packet[12..16] != b"REMB" {
        return None;
    }
    if packet.len() < 20 {
        return None;
    }
    let exp = (packet[17] >> 2) as u64;
    let mantissa = ((packet[17] as u64 & 0x03) << 16)
        | ((packet[18] as u64) << 8)
        | packet[19] as u64;
    Some(mantissa << exp)
}

#[cfg(test)]
mod remb_tests {
    use super::*;
    #[test]
    fn parses_remb() {
        let mut p = vec![0x8f, 206, 0, 4, 0, 0, 0, 1, 0, 0, 0, 0];
        p.extend_from_slice(b"REMB");
        p.push(1);
        p.extend_from_slice(&[0x40, 0x00, 0x01]);
        assert_eq!(parse_remb(&p), Some(1 << 16));
    }
}

use interceptor::stream_info::StreamInfo;
use interceptor::{Attributes, Interceptor, RTCPReader, RTCPWriter, RTPReader, RTPWriter};
use rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate;
use rtcp::transport_feedbacks::transport_layer_cc::{PacketStatusChunk, SymbolTypeTcc, TransportLayerCc};

#[derive(Clone)]
pub struct RembInterceptor {
    key: String,
}

impl RembInterceptor {
    pub fn new(key: &str) -> Self {
        Self { key: key.to_string() }
    }
}

struct RembReader {
    inner: std::sync::Arc<dyn RTCPReader + Send + Sync>,
    key: String,
}

#[async_trait::async_trait]
impl RTCPReader for RembReader {
    async fn read(
        &self,
        buf: &mut [u8],
        attributes: &Attributes,
    ) -> std::result::Result<(Vec<Box<dyn rtcp::packet::Packet + Send + Sync>>, Attributes), interceptor::Error> {
        let (pkts, attrs) = self.inner.read(buf, attributes).await?;
        for p in &pkts {
            if let Some(r) = p.as_any().downcast_ref::<ReceiverEstimatedMaximumBitrate>() {
                let rid = if r.bitrate < 300_000.0 { "l" } else if r.bitrate > 1_000_000.0 { "h" } else { "m" };
                set_rid(&self.key, rid);
            }
            if let Some(t) = p.as_any().downcast_ref::<TransportLayerCc>() {
                let (total, lost) = t.packet_chunks.iter().fold((0usize, 0usize), |(tot, l), c| {
                    if let PacketStatusChunk::StatusVectorChunk(v) = c {
                        let n = v.symbol_list.len();
                        let lost = v.symbol_list.iter().filter(|s| **s == SymbolTypeTcc::PacketNotReceived).count();
                        (tot + n, l + lost)
                    } else { (tot, l) }
                });
                if total > 0 { set_rid(&self.key, twcc_rid(total, lost)); }
                if let Some(bps) = twcc_bps(&self.key, t) { set_rid(&self.key, rid_for_bps(bps)); }
            }
        }
        Ok((pkts, attrs))
    }
}

struct RtpSendRecorder {
    inner: std::sync::Arc<dyn RTPWriter + Send + Sync>,
    key: String,
}

#[async_trait::async_trait]
impl RTPWriter for RtpSendRecorder {
    async fn write(
        &self,
        pkt: &rtp::packet::Packet,
        attributes: &Attributes,
    ) -> std::result::Result<usize, interceptor::Error> {
        let bytes = pkt.payload.len() + 12;
        twcc_record_sent(&self.key, pkt.header.sequence_number, bytes);
        self.inner.write(pkt, attributes).await
    }
}

#[async_trait::async_trait]
impl Interceptor for RembInterceptor {
    async fn bind_rtcp_reader(
        &self,
        reader: std::sync::Arc<dyn RTCPReader + Send + Sync>,
    ) -> std::sync::Arc<dyn RTCPReader + Send + Sync> {
        std::sync::Arc::new(RembReader { inner: reader, key: self.key.clone() })
    }

    async fn bind_rtcp_writer(
        &self,
        writer: std::sync::Arc<dyn RTCPWriter + Send + Sync>,
    ) -> std::sync::Arc<dyn RTCPWriter + Send + Sync> {
        writer
    }

    async fn bind_local_stream(
        &self,
        _info: &StreamInfo,
        writer: std::sync::Arc<dyn RTPWriter + Send + Sync>,
    ) -> std::sync::Arc<dyn RTPWriter + Send + Sync> {
        std::sync::Arc::new(RtpSendRecorder { inner: writer, key: self.key.clone() })
    }

    async fn unbind_local_stream(&self, _info: &StreamInfo) {}

    async fn bind_remote_stream(
        &self,
        _info: &StreamInfo,
        reader: std::sync::Arc<dyn RTPReader + Send + Sync>,
    ) -> std::sync::Arc<dyn RTPReader + Send + Sync> {
        reader
    }

    async fn unbind_remote_stream(&self, _info: &StreamInfo) {}

    async fn close(&self) -> std::result::Result<(), interceptor::Error> {
        Ok(())
    }
}

use interceptor::registry::Registry;
use interceptor::InterceptorBuilder;

struct RembBuilder {
    key: String,
}

impl InterceptorBuilder for RembBuilder {
    fn build(&self, _id: &str) -> std::result::Result<std::sync::Arc<dyn Interceptor + Send + Sync>, interceptor::Error> {
        Ok(std::sync::Arc::new(RembInterceptor::new(&self.key)))
    }
}

pub fn registry(key: &str) -> Registry {
    let mut r = Registry::new();
    r.add(Box::new(RembBuilder { key: key.to_string() }));
    r
}

#[derive(Default)]
pub struct TwccEstimator {
    sent: std::collections::HashMap<u16, SentPacket>,
    estimate_bps: u64,
}

#[derive(Clone, Copy)]
struct SentPacket {
    bytes: usize,
    sent_us: u64,
}

fn now_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

impl TwccEstimator {
    pub fn record_sent(&mut self, seq: u16, bytes: usize) {
        self.record_sent_at(seq, bytes, now_us());
    }

    /// Record an outgoing RTP packet with its send timestamp.
    pub fn record_sent_at(&mut self, seq: u16, bytes: usize, sent_us: u64) {
        if self.sent.len() > 8192 {
            self.sent.clear();
        }
        self.sent.insert(seq, SentPacket { bytes, sent_us });
    }

    /// Estimate bps from a list of `(sequence, arrival_us)` pairs.
    ///
    /// This is the per-packet path used by TWCC feedback. It uses the packet
    /// sizes and send timestamps recorded by the RTP writer, and the arrival
    /// timestamps reconstructed from `reference_time + recv_deltas`.
    pub fn estimate_arrivals(&mut self, arrivals: &[(u16, u64)]) -> Option<u64> {
        if arrivals.is_empty() {
            return None;
        }
        let mut bytes = 0u64;
        let mut first_arrival = u64::MAX;
        let mut last_arrival = 0u64;
        let mut first_send = u64::MAX;
        let mut last_send = 0u64;
        let mut known_send = 0u32;
        for (seq, arrival) in arrivals {
            first_arrival = first_arrival.min(*arrival);
            last_arrival = last_arrival.max(*arrival);
            if let Some(p) = self.sent.get(seq) {
                bytes += p.bytes as u64;
                first_send = first_send.min(p.sent_us);
                last_send = last_send.max(p.sent_us);
                known_send += 1;
            } else {
                // Conservative fallback for feedback for packets we did not see.
                bytes += 1200;
            }
        }
        if bytes == 0 || last_arrival <= first_arrival {
            return None;
        }
        let arrival_span = last_arrival - first_arrival;
        let instant = bytes.saturating_mul(8).saturating_mul(1_000_000) / arrival_span;
        let adjusted = if known_send > 0 && last_send > first_send {
            let send_span = last_send - first_send;
            // Positive queue delay means the path is building a backlog.
            if arrival_span > send_span.saturating_add(20_000) {
                instant.saturating_mul(2) / 3
            } else {
                instant
            }
        } else {
            instant
        };
        self.estimate_bps = if self.estimate_bps == 0 {
            adjusted
        } else {
            (self.estimate_bps.saturating_mul(7) + adjusted) / 8
        };
        Some(self.estimate_bps)
    }

    /// Legacy estimate used by unit tests / callers without arrival timestamps.
    pub fn estimate(&mut self, recv: &[u16], elapsed_us: u64) -> Option<u64> {
        if elapsed_us == 0 || recv.is_empty() {
            return None;
        }
        let bytes: u64 = recv
            .iter()
            .map(|s| self.sent.get(s).map(|p| p.bytes).unwrap_or(1200) as u64)
            .sum();
        Some(bytes * 8 * 1_000_000 / elapsed_us)
    }

    pub fn on_feedback(&mut self, fb: &TransportLayerCc) -> Option<u64> {
        let mut symbols = Vec::with_capacity(fb.packet_status_count as usize);
        for chunk in &fb.packet_chunks {
            match chunk {
                PacketStatusChunk::RunLengthChunk(c) => {
                    for _ in 0..c.run_length {
                        symbols.push(c.packet_status_symbol);
                    }
                }
                PacketStatusChunk::StatusVectorChunk(v) => {
                    symbols.extend(v.symbol_list.iter().copied())
                }
            }
        }
        let base_us = fb.reference_time as u64 * 64_000;
        let mut arrival_us = base_us;
        let mut recv_idx = 0usize;
        let mut arrivals = Vec::new();
        for (i, symbol) in symbols.iter().enumerate() {
            if *symbol == SymbolTypeTcc::PacketNotReceived {
                continue;
            }
            if let Some(delta) = fb.recv_deltas.get(recv_idx) {
                arrival_us = (arrival_us as i64).saturating_add(delta.delta) as u64;
            }
            recv_idx += 1;
            arrivals.push((fb.base_sequence_number.wrapping_add(i as u16), arrival_us));
        }
        self.estimate_arrivals(&arrivals)
    }
}

#[cfg(test)]
mod twcc_est_tests {
    use super::*;

    #[test]
    fn estimates_with_send_history() {
        let mut e = TwccEstimator::default();
        e.record_sent(10, 1000);
        e.record_sent(11, 2000);
        assert_eq!(e.estimate(&[10, 11], 100_000), Some(3000 * 8 * 1_000_000 / 100_000));
    }

    #[test]
    fn empty_feedback_is_none() {
        let mut e = TwccEstimator::default();
        assert!(e.estimate(&[], 1000).is_none());
        assert!(e.estimate(&[1], 0).is_none());
    }

    #[test]
    fn missing_history_uses_1200() {
        let mut e = TwccEstimator::default();
        assert_eq!(e.estimate(&[1, 2], 100_000), Some(2400 * 8 * 1_000_000 / 100_000));
    }

    #[test]
    fn per_packet_arrival_and_send_times() {
        let mut e = TwccEstimator::default();
        e.record_sent_at(10, 1000, 0);
        e.record_sent_at(11, 1000, 10_000);
        // 2000 bytes over 10 ms => 1.6 Mbps.
        assert_eq!(
            e.estimate_arrivals(&[(10, 1_000_000), (11, 1_010_000)]),
            Some(1_600_000)
        );
    }

    #[test]
    fn queue_delay_reduces_estimate() {
        let mut e = TwccEstimator::default();
        e.record_sent_at(20, 1000, 0);
        e.record_sent_at(21, 1000, 10_000);
        // Send span 10 ms, arrival span 100 ms: queue building, adjust down.
        let bps = e.estimate_arrivals(&[(20, 0), (21, 100_000)]).unwrap();
        assert_eq!(bps, 160_000 * 2 / 3);
    }

    #[test]
    fn feedback_parses_arrival_deltas() {
        let mut e = TwccEstimator::default();
        e.record_sent_at(10, 1000, 0);
        e.record_sent_at(11, 1000, 10_000);
        let fb = TransportLayerCc {
            base_sequence_number: 10,
            packet_status_count: 2,
            reference_time: 100,
            packet_chunks: vec![PacketStatusChunk::RunLengthChunk(
                rtcp::transport_feedbacks::transport_layer_cc::RunLengthChunk {
                    packet_status_symbol: SymbolTypeTcc::PacketReceivedSmallDelta,
                    run_length: 2,
                    ..Default::default()
                },
            )],
            recv_deltas: vec![
                rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                    delta: 0,
                    ..Default::default()
                },
                rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                    delta: 10_000,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(e.on_feedback(&fb), Some(1_600_000));
    }
}

static TWCC_EST: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, TwccEstimator>>> = std::sync::OnceLock::new();

pub fn twcc_bps(key: &str, fb: &TransportLayerCc) -> Option<u64> {
    let m = TWCC_EST.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut map = m.lock().unwrap_or_else(|e| e.into_inner());
    map.entry(key.to_string()).or_default().on_feedback(fb)
}

/// Record an outgoing RTP packet for TWCC estimation.
pub fn twcc_record_sent(key: &str, seq: u16, bytes: usize) {
    let m = TWCC_EST.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut map = m.lock().unwrap_or_else(|e| e.into_inner());
    map.entry(key.to_string()).or_default().record_sent(seq, bytes);
}

pub fn rid_for_bps(bps: u64) -> &'static str {
    if bps < 300_000 { "l" } else if bps > 1_000_000 { "h" } else { "m" }
}
