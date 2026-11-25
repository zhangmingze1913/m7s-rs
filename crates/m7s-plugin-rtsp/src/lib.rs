#![allow(clippy::collapsible_if)]
#![allow(clippy::items_after_test_module)]
//! RTSP SDP and RTCP helpers.
use base64::Engine;
use base64::engine::general_purpose::STANDARD;

pub fn sdp(sps: &[u8], pps: &[u8], asc: &[u8]) -> String {
    let mut out = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=m7s-rs\r\nt=0 0\r\na=control:*\r\n".to_string();
    if !sps.is_empty() {
        let fmtp = format!(
            "a=fmtp:96 packetization-mode=1;sprop-parameter-sets={},{}\r\n",
            STANDARD.encode(sps), STANDARD.encode(pps)
        );
        out.push_str(&format!("m=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n{fmtp}a=control:trackID=0\r\n"));
    }
    if !asc.is_empty() {
        let idx = ((asc[0] & 0x07) << 1) | (asc.get(1).copied().unwrap_or(0) >> 7);
        let rate = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350][idx as usize % 13];
        let channels = (asc.get(1).copied().unwrap_or(0) >> 3) & 0x0f;
        let hex: String = asc.iter().map(|b| format!("{b:02x}")).collect();
        out.push_str(&format!("m=audio 0 RTP/AVP 97\r\na=rtpmap:97 MPEG4-GENERIC/{rate}/{channels}\r\na=fmtp:97 streamtype=5;profile-level-id=1;mode=AAC-hbr;sizelength=13;indexlength=3;indexdeltalength=3;config={hex}\r\na=control:trackID=1\r\n"));
    }
    out
}

pub fn rtcp_sr(ssrc: u32, rtp_ts: u32) -> [u8; 28] {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let ntp_sec = now.as_secs() as u32 + 2_208_988_800;
    let ntp_frac = ((now.subsec_nanos() as u64) << 32) / 1_000_000_000;
    let mut p = [0u8; 28];
    p[0] = 0x80; p[1] = 200;
    p[2..4].copy_from_slice(&6u16.to_be_bytes());
    p[4..8].copy_from_slice(&ssrc.to_be_bytes());
    p[8..12].copy_from_slice(&ntp_sec.to_be_bytes());
    p[12..16].copy_from_slice(&(ntp_frac as u32).to_be_bytes());
    p[16..20].copy_from_slice(&rtp_ts.to_be_bytes());
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sdp_has_tracks() {
        let text = sdp(&[0x67, 1], &[0x68, 2], &[0x12, 0x10]);
        assert!(text.contains("H264/90000"));
        assert!(text.contains("MPEG4-GENERIC/44100/2"));
    }
    #[test]
    fn sr_header() {
        let p = rtcp_sr(1, 2);
        assert_eq!(p[1], 200);
        assert_eq!(&p[4..8], &[0, 0, 0, 1]);
    }
}

pub fn client_ports(headers: &str) -> Option<(u16, u16)> {
    let rest = headers.split("client_port=").nth(1)?;
    let token = rest.split([';', '\r', '\n', ' ']).next()?;
    let (a, b) = token.split_once('-')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

pub fn parse_path(url: &str) -> Option<(&str, &str)> {
    let rest = url.split("rtsp://").nth(1).unwrap_or(url);
    let path = rest.split('/').skip(1).collect::<Vec<_>>();
    if path.len() >= 2 { Some((path[0], path[1])) } else { None }
}

pub fn track_id(url: &str) -> u8 {
    url.split("trackID=")
        .nth(1)
        .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod path_tests {
    use super::*;
    #[test]
    fn parses() {
        assert_eq!(parse_path("rtsp://x/live/test"), Some(("live", "test")));
        assert_eq!(track_id("rtsp://x/live/test/trackID=1"), 1);
        assert_eq!(client_ports("client_port=1000-1001"), Some((1000, 1001)));
    }
}

pub mod server;
pub use server::{RtspAuth, audio_config, avc_config, run, run_with_auth, run_with_context};
