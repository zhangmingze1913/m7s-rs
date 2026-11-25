//! Foundation media types for m7s (zero internal deps).
#![allow(clippy::collapsible_if)]

use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct FlvTag {
    /// 8 = audio, 9 = video, 18 = script data.
    pub tag_type: u8,
    pub timestamp: u32,
    pub data: Vec<u8>,
    pub created: Instant,
}

impl FlvTag {
    pub fn new(tag_type: u8, timestamp: u32, data: Vec<u8>) -> Self {
        Self {
            tag_type,
            timestamp,
            data,
            created: Instant::now(),
        }
    }

    /// Full FLV tag including the 11-byte header and PreviousTagSize.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(11 + self.data.len() + 4);
        self.write_to(&mut out);
        out
    }

    /// Append the FLV tag to an existing buffer without allocating.
    pub fn write_to(&self, out: &mut Vec<u8>) {
        out.push(self.tag_type);
        out.extend_from_slice(&(self.data.len() as u32).to_be_bytes()[1..]);
        out.extend_from_slice(&self.timestamp.to_be_bytes()[1..]);
        out.push((self.timestamp >> 24) as u8);
        out.extend_from_slice(&[0, 0, 0]);
        out.extend_from_slice(&self.data);
        out.extend_from_slice(&((11 + self.data.len()) as u32).to_be_bytes());
    }

    pub fn from_rtmp(tag_type: u8, timestamp: u32, data: Vec<u8>) -> Self {
        Self::new(tag_type, timestamp, data)
    }
}

pub type SharedTag = Arc<FlvTag>;
pub mod ts;
pub use ts::TsMuxer;

#[derive(Debug, Clone)]
pub struct MediaFrame {
    pub track: TrackKind,
    pub sequence: u64,
    pub timestamp_ms: u32,
    pub keyframe: bool,
    pub data: Arc<[u8]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
    Metadata,
}

impl TrackKind {
    pub fn from_tag(tag: &FlvTag) -> Self {
        match tag.tag_type {
            9 => Self::Video,
            8 => Self::Audio,
            _ => Self::Metadata,
        }
    }
}

impl MediaFrame {
    pub fn from_flv(tag: &FlvTag, sequence: u64) -> Self {
        let track = TrackKind::from_tag(tag);
        let keyframe = track == TrackKind::Video && tag.data.first().is_some_and(|b| (b >> 4) == 1);
        Self {
            track,
            sequence,
            timestamp_ms: tag.timestamp,
            keyframe,
            data: Arc::from(tag.data.clone()),
        }
    }

    pub fn is_video(&self) -> bool {
        self.track == TrackKind::Video
    }

    pub fn is_audio(&self) -> bool {
        self.track == TrackKind::Audio
    }
}

/// Extract SPS/PPS from an FLV AVC sequence header in a GOP snapshot.
pub fn avc_config(cache: &[SharedTag]) -> (Vec<u8>, Vec<u8>) {
    for tag in cache {
        if tag.tag_type != 9 || tag.data.len() < 11 || tag.data[1] != 0 {
            continue;
        }
        let data = &tag.data[5..];
        let mut pos = 6;
        let num_sps = (data[5] & 0x1f) as usize;
        let mut sps = Vec::new();
        for _ in 0..num_sps {
            if pos + 2 > data.len() {
                break;
            }
            let len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;
            if pos + len <= data.len() {
                sps = data[pos..pos + len].to_vec();
                pos += len;
            }
        }
        if pos >= data.len() {
            continue;
        }
        let num_pps = data[pos] as usize;
        pos += 1;
        for _ in 0..num_pps {
            if pos + 2 > data.len() {
                break;
            }
            let len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;
            if pos + len <= data.len() {
                return (sps, data[pos..pos + len].to_vec());
            }
        }
    }
    (Vec::new(), Vec::new())
}

/// Extract AAC AudioSpecificConfig from an FLV audio sequence header.
pub fn audio_config(cache: &[SharedTag]) -> Vec<u8> {
    for tag in cache {
        if tag.tag_type == 8 && tag.data.len() > 2 && tag.data[1] == 0 {
            return tag.data[2..].to_vec();
        }
    }
    Vec::new()
}

pub fn flv_header(has_audio: bool, has_video: bool) -> [u8; 13] {
    let mut out = [0u8; 13];
    out[0] = b'F';
    out[1] = b'L';
    out[2] = b'V';
    out[3] = 1;
    out[4] = (has_audio as u8) << 2 | (has_video as u8);
    out[5..9].copy_from_slice(&9u32.to_be_bytes());
    out
}

#[derive(Default)]
pub struct FlvReader {
    buffer: Vec<u8>,
    header_done: bool,
}

impl FlvReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    pub fn next_tag(&mut self) -> Result<Option<FlvTag>, anyhow::Error> {
        if !self.header_done {
            if self.buffer.len() < 13 {
                return Ok(None);
            }
            if &self.buffer[0..3] != b"FLV" {
                anyhow::bail!("invalid FLV header");
            }
            self.buffer.drain(..13);
            self.header_done = true;
        }
        loop {
            if self.buffer.len() < 15 {
                return Ok(None);
            }
            let tag_type = self.buffer[0];
            let size =
                u32::from_be_bytes([0, self.buffer[1], self.buffer[2], self.buffer[3]]) as usize;
            let timestamp = u32::from_be_bytes([
                self.buffer[7],
                self.buffer[4],
                self.buffer[5],
                self.buffer[6],
            ]);
            let total = 11 + size + 4;
            if self.buffer.len() < total {
                return Ok(None);
            }
            let data = self.buffer[11..11 + size].to_vec();
            self.buffer.drain(..total);
            if tag_type == 8 || tag_type == 9 || tag_type == 18 {
                return Ok(Some(FlvTag {
                    created: Instant::now(),
                    tag_type,
                    timestamp,
                    data,
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flv_roundtrip_size() {
        let tag = FlvTag::new(9, 0x01020304, vec![1, 2, 3]);
        let bytes = tag.to_bytes();
        assert_eq!(bytes.len(), 11 + 3 + 4);
        assert_eq!(bytes[0], 9);
        assert_eq!(&bytes[1..4], &[0, 0, 3]);
        assert_eq!(bytes[4..7], [0x02, 0x03, 0x04]);
        assert_eq!(bytes[7], 0x01);
    }

    #[test]
    fn keyframe_detection() {
        let key = FlvTag::new(9, 0, vec![0x17, 1]);
        let frame = MediaFrame::from_flv(&key, 1);
        assert!(frame.keyframe);
        assert!(frame.is_video());
        let audio = MediaFrame::from_flv(&FlvTag::new(8, 0, vec![0xaf, 1]), 2);
        assert!(audio.is_audio());
    }
}

/// Split Annex B by 3/4-byte start codes.
pub fn split_annexb(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut start: Option<usize> = None;
    while i + 3 <= data.len() {
        let sc = if data[i..].starts_with(&[0, 0, 1]) {
            3
        } else if data[i..].starts_with(&[0, 0, 0, 1]) {
            4
        } else {
            0
        };
        if sc > 0 {
            if let Some(s) = start {
                if i > s {
                    out.push(&data[s..i]);
                }
            }
            start = Some(i + sc);
            i += sc;
        } else {
            i += 1;
        }
    }
    if let Some(s) = start {
        if s < data.len() {
            out.push(&data[s..]);
        }
    }
    out
}

/// Convert Annex B NAL units to AVCC length-prefixed form.
pub fn annexb_to_avcc(annexb: &[u8]) -> (bool, Vec<u8>) {
    let mut out = Vec::new();
    let mut key = false;
    for nal in split_annexb(annexb) {
        if nal.is_empty() {
            continue;
        }
        if nal[0] & 0x1f == 5 {
            key = true;
        }
        out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        out.extend_from_slice(nal);
    }
    (key, out)
}

/// Convert AVCC length-prefixed NAL units to Annex B.
pub fn avcc_to_annexb(avcc: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= avcc.len() {
        let len = u32::from_be_bytes([avcc[i], avcc[i + 1], avcc[i + 2], avcc[i + 3]]) as usize;
        i += 4;
        if len == 0 || i + len > avcc.len() {
            break;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&avcc[i..i + len]);
        i += len;
    }
    out
}

#[cfg(test)]
mod nal_tests {
    use super::*;

    #[test]
    fn annexb_avcc_roundtrip() {
        let annexb = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 1, 0x65, 4, 5];
        let (key, avcc) = annexb_to_avcc(&annexb);
        assert!(key);
        assert_eq!(avcc.len(), 3 * 4 + 3 + 2 + 3);
        let back = avcc_to_annexb(&avcc);
        assert_eq!(split_annexb(&back).len(), 3);
        assert_eq!(split_annexb(&back)[2], &[0x65, 4, 5]);
    }
}

/// Parse one ADTS frame into raw AAC + AudioSpecificConfig.
pub fn adts_to_raw(frame: &[u8]) -> Option<(Vec<u8>, [u8; 2])> {
    if frame.len() < 7 || frame[0] != 0xff || frame[1] & 0xf0 != 0xf0 {
        return None;
    }
    let profile = (frame[2] >> 6) & 0x03;
    let freq_idx = (frame[2] >> 2) & 0x0f;
    let channels = ((frame[2] & 0x01) << 2) | ((frame[3] >> 6) & 0x03);
    let protection_absent = frame[1] & 0x01;
    let header = if protection_absent != 0 { 7 } else { 9 };
    let len = ((usize::from(frame[3]) & 0x03) << 11) | (usize::from(frame[4]) << 3) | (usize::from(frame[5]) >> 5);
    if len < header || len > frame.len() {
        return None;
    }
    let asc = [((profile + 1) << 3) | (freq_idx >> 1), ((freq_idx & 1) << 7) | (channels << 3)];
    Some((frame[header..len].to_vec(), asc))
}

/// Wrap raw AAC into an ADTS frame.
pub fn raw_to_adts(raw: &[u8], asc: &[u8; 2]) -> Vec<u8> {
    let profile = (asc[0] >> 3).saturating_sub(1) & 0x03;
    let freq_idx = ((asc[0] & 0x07) << 1) | (asc[1] >> 7);
    let channels = (asc[1] >> 3) & 0x0f;
    let len = raw.len() + 7;
    let mut out = vec![0u8; 7];
    out[0] = 0xff;
    out[1] = 0xf1;
    out[2] = (profile << 6) | ((freq_idx & 0x0f) << 2) | ((channels >> 2) & 0x01);
    out[3] = ((channels & 0x03) << 6) | (((len >> 11) & 0x03) as u8);
    out[4] = ((len >> 3) & 0xff) as u8;
    out[5] = (((len & 0x07) << 5) as u8) | 0x1f;
    out[6] = 0xfc;
    out.extend_from_slice(raw);
    out
}

#[cfg(test)]
mod adts_tests {
    use super::*;

    #[test]
    fn adts_roundtrip() {
        let asc = [0x12, 0x10];
        let raw = [1u8, 2, 3, 4, 5];
        let frame = raw_to_adts(&raw, &asc);
        let (got, got_asc) = adts_to_raw(&frame).unwrap();
        assert_eq!(got, raw);
        assert_eq!(got_asc[0] >> 3, asc[0] >> 3);
        assert_eq!(got_asc[1] >> 3 & 0x0f, asc[1] >> 3 & 0x0f);
    }
}

/// Build an AVCDecoderConfigurationRecord from SPS/PPS.
pub fn build_avc_config(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(11 + sps.len() + pps.len());
    out.push(1);
    out.push(*sps.first().unwrap_or(&0x42));
    out.push(*sps.get(1).unwrap_or(&0));
    out.push(*sps.get(2).unwrap_or(&0x1e));
    out.push(0xff);
    out.push(0xe1);
    out.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    out.extend_from_slice(sps);
    out.push(1);
    out.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    out.extend_from_slice(pps);
    out
}

/// Parse SPS/PPS from an AVCDecoderConfigurationRecord.
pub fn parse_avc_config(record: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    if record.len() < 7 || record[0] != 1 {
        return None;
    }
    let mut pos = 5;
    let num_sps = record[pos] & 0x1f;
    pos += 1;
    let mut sps = Vec::new();
    for _ in 0..num_sps {
        if pos + 2 > record.len() {
            return None;
        }
        let len = u16::from_be_bytes([record[pos], record[pos + 1]]) as usize;
        pos += 2;
        if pos + len > record.len() {
            return None;
        }
        sps = record[pos..pos + len].to_vec();
        pos += len;
    }
    if pos >= record.len() {
        return None;
    }
    let num_pps = record[pos];
    pos += 1;
    let mut pps = Vec::new();
    for _ in 0..num_pps {
        if pos + 2 > record.len() {
            return None;
        }
        let len = u16::from_be_bytes([record[pos], record[pos + 1]]) as usize;
        pos += 2;
        if pos + len > record.len() {
            return None;
        }
        pps = record[pos..pos + len].to_vec();
        pos += len;
    }
    Some((sps, pps))
}

#[cfg(test)]
mod avc_config_tests {
    use super::*;

    #[test]
    fn avc_config_roundtrip() {
        let sps = [0x67, 0x42, 0x00, 0x1e, 0x95, 0xa8, 0x28, 0x0f];
        let pps = [0x68, 0xce, 0x3c, 0x80];
        let rec = build_avc_config(&sps, &pps);
        let (s, p) = parse_avc_config(&rec).unwrap();
        assert_eq!(s, sps);
        assert_eq!(p, pps);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SvcHeader {
    pub idr: bool,
    pub priority_id: u8,
    pub no_inter_layer_pred: bool,
    pub dependency_id: u8,
    pub quality_id: u8,
    pub temporal_id: u8,
    pub use_ref_base_pic: bool,
    pub discardable: bool,
    pub output: bool,
}

/// Parse the 3-byte SVC extension header of NAL types 14/20.
pub fn parse_svc_header(nal: &[u8]) -> Option<SvcHeader> {
    let nal_type = nal.first()? & 0x1f;
    if nal_type != 14 && nal_type != 20 {
        return None;
    }
    let b1 = *nal.get(1)?;
    let b2 = *nal.get(2)?;
    let b3 = *nal.get(3)?;
    Some(SvcHeader {
        idr: nal_type == 20 && (nal[0] & 0x80) != 0,
        priority_id: b1 & 0x3f,
        no_inter_layer_pred: b1 & 0x40 != 0,
        dependency_id: (b2 >> 4) & 0x07,
        quality_id: b2 & 0x0f,
        temporal_id: (b3 >> 5) & 0x07,
        use_ref_base_pic: b3 & 0x10 != 0,
        discardable: b3 & 0x08 != 0,
        output: b3 & 0x04 != 0,
    })
}

pub fn is_svc(nal: &[u8]) -> bool {
    nal.first().is_some_and(|b| matches!(b & 0x1f, 14 | 20))
}

#[cfg(test)]
mod svc_tests {
    use super::*;

    #[test]
    fn parses_svc_extension() {
        let nal = [0x74, 0x45, 0x23, 0x34];
        assert!(is_svc(&nal));
        let h = parse_svc_header(&nal).unwrap();
        assert!(h.no_inter_layer_pred);
        assert_eq!(h.priority_id, 5);
        assert_eq!(h.dependency_id, 2);
        assert_eq!(h.quality_id, 3);
        assert_eq!(h.temporal_id, 1);
        assert!(h.output);
    }
}

/// Keep base layer and SVC NALs up to `max_dependency` from an Annex B access unit.
pub fn svc_select_layers(annexb: &[u8], max_dependency: u8) -> Vec<u8> {
    let mut out = Vec::new();
    for nal in split_annexb(annexb) {
        if let Some(h) = parse_svc_header(nal) {
            if h.dependency_id > max_dependency {
                continue;
            }
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
    }
    out
}

#[cfg(test)]
mod svc_filter_tests {
    use super::*;

    #[test]
    fn drops_enhancement_layers() {
        let base = [0u8, 0, 0, 1, 0x65, 0xaa];
        let dep0 = [0u8, 0, 0, 1, 0x74, 0x45, 0x03, 0x24];
        let dep2 = [0u8, 0, 0, 1, 0x74, 0x45, 0x23, 0x24];
        let mut au = Vec::new();
        au.extend_from_slice(&base);
        au.extend_from_slice(&dep0);
        au.extend_from_slice(&dep2);
        let filtered = svc_select_layers(&au, 0);
        let nals = split_annexb(&filtered);
        assert_eq!(nals.len(), 2);
        assert_eq!(nals[1], &[0x74, 0x45, 0x03, 0x24]);
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }
    fn bit(&mut self) -> Option<u32> {
        if self.bit / 8 >= self.data.len() {
            return None;
        }
        let v = (self.data[self.bit / 8] >> (7 - (self.bit % 8))) & 1;
        self.bit += 1;
        Some(v as u32)
    }
    fn bits(&mut self, n: usize) -> Option<u32> {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Some(v)
    }
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        let rest = if zeros == 0 { 0 } else { self.bits(zeros)? };
        Some((1u32 << zeros) - 1 + rest)
    }
    fn se(&mut self) -> Option<i32> {
        let v = self.ue()? as i64;
        Some(if v & 1 == 1 { ((v + 1) / 2) as i32 } else { -(v / 2) as i32 })
    }
}

/// Parse width/height from an H.264 SPS NAL (with or without Annex B prefix).
pub fn sps_dimensions(sps: &[u8]) -> Option<(u32, u32)> {
    let mut raw = Vec::new();
    let mut i = 0;
    if sps.starts_with(&[0, 0, 0, 1]) {
        i = 4;
    } else if sps.starts_with(&[0, 0, 1]) {
        i = 3;
    }
    let mut zeros = 0;
    while i < sps.len() {
        let b = sps[i];
        if zeros >= 2 && b == 3 {
            zeros = 0;
            i += 1;
            continue;
        }
        raw.push(b);
        zeros = if b == 0 { zeros + 1 } else { 0 };
        i += 1;
    }
    if raw.is_empty() || raw[0] & 0x1f != 7 {
        return None;
    }
    let mut r = BitReader::new(&raw[1..]);
    let profile = r.bits(8)?;
    r.bits(8)?;
    r.bits(8)?;
    r.ue()?;
    let mut chroma = 1u32;
    if matches!(profile, 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135) {
        chroma = r.ue()?;
        if chroma == 3 {
            r.bit()?;
        }
        r.ue()?;
        r.ue()?;
        r.bit()?;
        if r.bit()? == 1 {
            let n = if chroma != 3 { 8 } else { 12 };
            for i in 0..n {
                if r.bit()? == 1 {
                    let mut last = 8i32;
                    let mut next = 8i32;
                    let size = if i < 6 { 16 } else { 64 };
                    for _ in 0..size {
                        if next != 0 {
                            next = (last + r.se()? + 256) % 256;
                        }
                        last = if next == 0 { last } else { next };
                    }
                }
            }
        }
    }
    r.ue()?;
    let poc = r.ue()?;
    if poc == 0 {
        r.ue()?;
    } else if poc == 1 {
        r.bit()?;
        r.se()?;
        r.se()?;
        let n = r.ue()?;
        for _ in 0..n {
            r.se()?;
        }
    }
    r.ue()?;
    r.bit()?;
    let w_mbs = r.ue()? + 1;
    let h_map = r.ue()? + 1;
    let frame_mbs_only = r.bit()?;
    if frame_mbs_only == 0 {
        r.bit()?;
    }
    r.bit()?;
    let mut crop_l = 0;
    let mut crop_r = 0;
    let mut crop_t = 0;
    let mut crop_b = 0;
    if r.bit()? == 1 {
        crop_l = r.ue()?;
        crop_r = r.ue()?;
        crop_t = r.ue()?;
        crop_b = r.ue()?;
    }
    let sub_w = if chroma == 3 { 1 } else { 2 };
    let sub_h = if chroma == 1 { 2 } else { 1 };
    let crop_x = sub_w;
    let crop_y = sub_h * (2 - frame_mbs_only);
    let width = w_mbs * 16 - (crop_l + crop_r) * crop_x;
    let height = (2 - frame_mbs_only) * h_map * 16 - (crop_t + crop_b) * crop_y;
    Some((width, height))
}

#[cfg(test)]
mod sps_tests {
    use super::*;

    #[test]
    fn parse_generated_sps() {
        let ff = std::env::var("M7S_FFMPEG").unwrap_or_else(|_| {
            let local = format!("{}/../../node_modules/ffmpeg-static/ffmpeg.exe", env!("CARGO_MANIFEST_DIR"));
            if std::path::Path::new(&local).exists() { local } else { "ffmpeg".into() }
        });
        let path = std::path::PathBuf::from(format!("{}/../../target/sps-test-{}.h264", env!("CARGO_MANIFEST_DIR"), std::process::id()));
        let ok = std::process::Command::new(&ff)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=1", "-frames:v", "1", "-c:v", "libx264", "-x264-params", "keyint=1", "-f", "h264"])
            .arg(&path)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return;
        }
        let data = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let sps = split_annexb(&data).into_iter().find(|n| n[0] & 0x1f == 7).unwrap();
        assert_eq!(sps_dimensions(sps), Some((320, 240)));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioSpecificConfig {
    pub object_type: u8,
    pub sample_rate: u32,
    pub channels: u8,
}

const AAC_RATES: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];

pub fn parse_asc(asc: &[u8]) -> Option<AudioSpecificConfig> {
    if asc.len() < 2 {
        return None;
    }
    let object_type = (asc[0] >> 3) & 0x1f;
    let idx = (((asc[0] & 0x07) as usize) << 1) | ((asc[1] >> 7) as usize);
    let channels = (asc[1] >> 3) & 0x0f;
    if object_type == 0 || idx >= AAC_RATES.len() || channels == 0 {
        return None;
    }
    Some(AudioSpecificConfig {
        object_type,
        sample_rate: AAC_RATES[idx],
        channels,
    })
}

pub fn build_asc(cfg: AudioSpecificConfig) -> Option<[u8; 2]> {
    let idx = AAC_RATES.iter().position(|r| *r == cfg.sample_rate)? as u8;
    Some([(cfg.object_type << 3) | (idx >> 1), ((idx & 1) << 7) | (cfg.channels << 3)])
}

#[cfg(test)]
mod asc_tests {
    use super::*;

    #[test]
    fn asc_roundtrip() {
        let cfg = AudioSpecificConfig { object_type: 2, sample_rate: 44100, channels: 2 };
        let asc = build_asc(cfg).unwrap();
        assert_eq!(parse_asc(&asc), Some(cfg));
    }
}

pub fn sps_profile_level(sps: &[u8]) -> Option<(u8, u8)> {
    let start = if sps.starts_with(&[0, 0, 0, 1]) {
        4
    } else if sps.starts_with(&[0, 0, 1]) {
        3
    } else {
        0
    };
    if sps.len() < start + 4 || sps[start] & 0x1f != 7 {
        return None;
    }
    Some((sps[start + 1], sps[start + 3]))
}

/// Parse VUI timing frame rate from an SPS.
pub fn sps_frame_rate(sps: &[u8]) -> Option<f64> {
    let mut raw = Vec::new();
    let mut i = 0;
    if sps.starts_with(&[0, 0, 0, 1]) { i = 4; } else if sps.starts_with(&[0, 0, 1]) { i = 3; }
    let mut zeros = 0;
    while i < sps.len() {
        let b = sps[i];
        if zeros >= 2 && b == 3 { zeros = 0; i += 1; continue; }
        raw.push(b);
        zeros = if b == 0 { zeros + 1 } else { 0 };
        i += 1;
    }
    if raw.len() < 4 || raw[0] & 0x1f != 7 { return None; }
    let mut r = BitReader::new(&raw[1..]);
    let profile = r.bits(8)?;
    r.bits(8)?;
    r.bits(8)?;
    r.ue()?;

    if matches!(profile, 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135) {
        let chroma = r.ue()?;
        if chroma == 3 { r.bit()?; }
        r.ue()?;
        r.ue()?;
        r.bit()?;
        if r.bit()? == 1 {
            let n = if chroma != 3 { 8 } else { 12 };
            for i in 0..n {
                if r.bit()? == 1 {
                    let mut last = 8i32;
                    let mut next = 8i32;
                    let size = if i < 6 { 16 } else { 64 };
                    for _ in 0..size {
                        if next != 0 { next = (last + r.se()? + 256) % 256; }
                        last = if next == 0 { last } else { next };
                    }
                }
            }
        }
    }
    r.ue()?;
    let poc = r.ue()?;
    if poc == 0 { r.ue()?; }
    else if poc == 1 {
        r.bit()?;
        r.se()?;
        r.se()?;
        let n = r.ue()?;
        for _ in 0..n { r.se()?; }
    }
    r.ue()?;
    r.bit()?;
    r.ue()?;
    r.ue()?;
    let frame_mbs_only = r.bit()?;
    if frame_mbs_only == 0 { r.bit()?; }
    r.bit()?;
    if r.bit()? == 1 {
        r.ue()?; r.ue()?; r.ue()?; r.ue()?;
    }
    if r.bit()? == 0 { return None; }
    if r.bit()? == 1 {
        r.bits(8)?;
        if r.bits(8)? == 255 { r.bits(16)?; r.bits(16)?; }
    }
    if r.bit()? == 1 { r.bit()?; }
    if r.bit()? == 1 {
        r.bits(3)?;
        r.bit()?;
        if r.bit()? == 1 { r.bits(8)?; r.bits(8)?; r.bits(8)?; }
    }
    if r.bit()? == 1 { r.ue()?; r.ue()?; }
    if r.bit()? == 0 { return None; }
    let num_units = r.bits(32)? as f64;
    let time_scale = r.bits(32)? as f64;
    r.bit()?;
    if num_units == 0.0 { return None; }
    Some(time_scale / (2.0 * num_units))
}

#[cfg(test)]
mod fps_tests {
    use super::*;

    struct Bw { bits: Vec<u8> }
    impl Bw {
        fn new() -> Self { Self { bits: Vec::new() } }
        fn bit(&mut self, v: u32) { self.bits.push((v & 1) as u8); }
        fn bits(&mut self, v: u32, n: usize) {
            for i in (0..n).rev() { self.bit((v >> i) & 1); }
        }
        fn ue(&mut self, v: u32) {
            let n = if v == 0 { 1 } else { 32 - v.leading_zeros() };
            for _ in 0..n - 1 { self.bit(0); }
            self.bits(v + 1, n as usize);
        }
        fn finish(mut self) -> Vec<u8> {
            self.bit(1);
            while self.bits.len() & 7 != 0 { self.bit(0); }
            let mut out = vec![0x67];
            for chunk in self.bits.chunks(8) {
                let mut b = 0u8;
                for bit in chunk { b = (b << 1) | bit; }
                out.push(b);
            }
            out
        }
    }

    #[test]
    fn synthetic_vui_fps_and_dims() {
        let mut w = Bw::new();
        w.bits(66, 8);
        w.bits(0, 8);
        w.bits(30, 8);
        w.ue(0);
        w.ue(0);
        w.ue(0);
        w.ue(0);
        w.ue(0);
        w.bit(0);
        w.ue(19);
        w.ue(14);
        w.bit(1);
        w.bit(0);
        w.bit(0);
        w.bit(1);
        w.bit(0);
        w.bit(0);
        w.bit(0);
        w.bit(0);
        w.bit(1);
        w.bits(1, 32);
        w.bits(50, 32);
        w.bit(1);
        let sps = w.finish();
        assert_eq!(sps_dimensions(&sps), Some((320, 240)));
        let fps = sps_frame_rate(&sps).unwrap();
        assert!((fps - 25.0).abs() < 0.01, "{fps}");
    }
}

#[cfg(test)]
mod flv_write_tests {
    use super::*;

    #[test]
    fn write_to_matches_to_bytes() {
        let tag = FlvTag::new(9, 1234, vec![1, 2, 3, 4]);
        let mut out = Vec::new();
        tag.write_to(&mut out);
        assert_eq!(out, tag.to_bytes());
    }
}
