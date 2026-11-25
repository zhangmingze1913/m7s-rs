pub mod fmp4;

// HLS playlist generation.
#[derive(Debug, Clone, Copy)]
pub struct PlaylistSegment {
    pub seq: u64,
    pub duration: f64,
}

pub fn playlist(segments: &[PlaylistSegment], target_secs: u64) -> String {
    let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:3\n");
    out.push_str(&format!("#EXT-X-TARGETDURATION:{}\n", target_secs.max(1)));
    out.push_str(&format!(
        "#EXT-X-MEDIA-SEQUENCE:{}\n",
        segments.first().map(|s| s.seq).unwrap_or(0)
    ));
    for segment in segments {
        out.push_str(&format!(
            "#EXTINF:{:.3},\nseg-{}.ts\n",
            segment.duration, segment.seq
        ));
    }
    out
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub bandwidth: u32,
    pub resolution: String,
    pub uri: String,
}

/// Multivariant (master) playlist.
pub fn master_playlist(variants: &[Variant]) -> String {
    let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:3\n");
    for v in variants {
        out.push_str(&format!(
            "#EXT-X-STREAM-INF:BANDWIDTH={},RESOLUTION={}\n{}\n",
            v.bandwidth, v.resolution, v.uri
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renders() {
        let text = playlist(&[PlaylistSegment { seq: 3, duration: 2.0 }], 2);
        assert!(text.contains("#EXTM3U"));
        assert!(text.contains("#EXT-X-MEDIA-SEQUENCE:3"));
        assert!(text.contains("seg-3.ts"));
    }

    #[test]
    fn master() {
        let text = master_playlist(&[Variant {
            bandwidth: 800_000,
            resolution: "640x360".into(),
            uri: "low.m3u8".into(),
        }]);
        assert!(text.contains("BANDWIDTH=800000"));
        assert!(text.contains("low.m3u8"));
    }
}

// HLS segmenter.
use m7s_codec::TsMuxer;

#[derive(Debug, Clone)]
pub struct Output { pub seq: u64, pub duration: f64, pub data: Vec<u8> }
pub struct Segmenter {
    mux: TsMuxer,
    buffer: Vec<u8>,
    enabled: bool,
    seq: u64,
    seg_start_ms: u32,
    seg_start_set: bool,
    target_ms: u32,
    sps_pps: Vec<u8>,
    nal_len: usize,
    asc: Vec<u8>,
    last_audio_pts: u64,
}

impl Segmenter {
    pub fn new(target_secs: u64) -> Self {
        Self {
            mux: TsMuxer::new(),
            buffer: Vec::new(),
            enabled: false,
            seq: 0,
            seg_start_ms: 0,
            seg_start_set: false,
            target_ms: (target_secs * 1000) as u32,
            sps_pps: Vec::new(),
            nal_len: 4,
            asc: Vec::new(),
            last_audio_pts: 0,
        }
    }

    pub fn push(&mut self, tag: &m7s_sdk::Tag, out: &mut Vec<Output>) {
        match tag.tag_type {
            9 => self.video(tag, out),
            8 => self.audio(tag, out),
            _ => {}
        }
    }

    fn video(&mut self, tag: &m7s_sdk::Tag, out: &mut Vec<Output>) {
        if tag.data.len() < 5 || (tag.data[0] & 0x0f) != 7 {
            return;
        }
        let keyframe = (tag.data[0] >> 4) == 1;
        match tag.data[1] {
            0 => self.parse_avc_config(&tag.data[5..]),
            1 => {
                if keyframe
                    && self.seg_start_set
                    && self.enabled
                    && tag.timestamp.wrapping_sub(self.seg_start_ms) >= self.target_ms
                    && !self.buffer.is_empty()
                {
                    self.finalize(out);
                }
                if !self.enabled {
                    self.enabled = true;
                    self.seg_start_ms = tag.timestamp;
                    self.seg_start_set = true;
                    self.buffer.clear();
                    self.mux.write_tables(&mut self.buffer);
                }
                let mut annex = Vec::with_capacity(tag.data.len() + self.sps_pps.len());
                if keyframe {
                    annex.extend_from_slice(&self.sps_pps);
                }
                let mut pos = 5;
                while pos + self.nal_len <= tag.data.len() {
                    let len = read_nal_len(&tag.data[pos..], self.nal_len);
                    pos += self.nal_len;
                    if len == 0 || pos + len > tag.data.len() {
                        break;
                    }
                    annex.extend_from_slice(&[0, 0, 0, 1]);
                    annex.extend_from_slice(&tag.data[pos..pos + len]);
                    pos += len;
                }
                let cts = read_cts(&tag.data[2..5]);
                let dts = tag.timestamp as u64 * 90;
                let pts = ((tag.timestamp as i64 + cts as i64).max(0) as u64) * 90;
                self.mux.write_video(&mut self.buffer, &annex, pts, dts);
            }
            _ => {}
        }
    }

    fn audio(&mut self, tag: &m7s_sdk::Tag, out: &mut Vec<Output>) {
        if tag.data.len() < 2 || (tag.data[0] >> 4) != 10 {
            return;
        }
        match tag.data[1] {
            0 => self.asc = tag.data[2..].to_vec(),
            1 if !self.asc.is_empty() => {
                if !self.enabled {
                    self.enabled = true;
                    self.seg_start_ms = tag.timestamp;
                    self.seg_start_set = true;
                    self.buffer.clear();
                    self.mux.write_tables(&mut self.buffer);
                } else if tag.timestamp.wrapping_sub(self.seg_start_ms) >= self.target_ms
                    && !self.buffer.is_empty()
                {
                    self.finalize(out);
                    self.enabled = true;
                    self.seg_start_ms = tag.timestamp;
                    self.seg_start_set = true;
                    self.buffer.clear();
                    self.mux.write_tables(&mut self.buffer);
                }
                let adts = adts_frame(&self.asc, &tag.data[2..]);
                let mut pts = tag.timestamp as u64 * 90;
                if pts <= self.last_audio_pts {
                    pts = self.last_audio_pts + 1;
                }
                self.last_audio_pts = pts;
                self.mux.write_audio(&mut self.buffer, &adts, pts);
            }
            _ => {}
        }
    }

    fn parse_avc_config(&mut self, data: &[u8]) {
        if data.len() < 7 {
            return;
        }
        self.nal_len = (data[4] & 0x03) as usize + 1;
        let mut pos = 6;
        let num_sps = (data[5] & 0x1f) as usize;
        let mut out = Vec::new();
        for _ in 0..num_sps {
            if pos + 2 > data.len() {
                return;
            }
            let len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;
            if pos + len > data.len() {
                return;
            }
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(&data[pos..pos + len]);
            pos += len;
        }
        if pos >= data.len() {
            return;
        }
        let num_pps = data[pos] as usize;
        pos += 1;
        for _ in 0..num_pps {
            if pos + 2 > data.len() {
                return;
            }
            let len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;
            if pos + len > data.len() {
                return;
            }
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(&data[pos..pos + len]);
            pos += len;
        }
        self.sps_pps = out;
    }

    fn finalize(&mut self, out: &mut Vec<Output>) {
        if self.buffer.is_empty() {
            return;
        }
        let duration = (self.target_ms as f64 / 1000.0).max(0.1);
        out.push(Output {
            seq: self.seq,
            duration,
            data: std::mem::take(&mut self.buffer),
        });
        self.seq += 1;
        self.mux = TsMuxer::new();
        self.enabled = false;
        self.seg_start_set = false;
    }
}

fn read_nal_len(data: &[u8], n: usize) -> usize {
    match n {
        1 => data[0] as usize,
        2 => u16::from_be_bytes([data[0], data[1]]) as usize,
        3 => u32::from_be_bytes([0, data[0], data[1], data[2]]) as usize,
        _ => u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize,
    }
}

fn read_cts(data: &[u8]) -> i32 {
    let v = u32::from_be_bytes([0, data[0], data[1], data[2]]);
    if v & 0x80_0000 != 0 {
        (v | 0xff00_0000) as i32
    } else {
        v as i32
    }
}

pub fn adts_frame(asc: &[u8], frame: &[u8]) -> Vec<u8> {
    let object_type = asc.first().map(|b| (b >> 3) & 0x1f).unwrap_or(2).max(1);
    let profile = (object_type - 1).min(3);
    let freq = asc
        .first()
        .map(|b| ((b & 0x07) << 1) | (asc.get(1).copied().unwrap_or(0) >> 7))
        .unwrap_or(4);
    let channel = asc.get(1).map(|b| (b >> 3) & 0x0f).unwrap_or(2).max(1);
    let len = frame.len() + 7;
    let mut out = Vec::with_capacity(len);
    out.push(0xff);
    out.push(0xf1);
    out.push((profile << 6) | (freq << 2) | (channel >> 2));
    out.push(((channel & 3) << 6) | ((len >> 11) as u8 & 0x03));
    out.push((len >> 3) as u8);
    out.push((((len & 0x07) as u8) << 5) | 0x1f);
    out.push(0xfc);
    out.extend_from_slice(frame);
    out
}


#[cfg(test)]
mod segment_tests {
    use super::*;
    use m7s_sdk::Tag;

    #[test]
    fn segmenter_emits_ts() {
        let mut seg = Segmenter::new(1);
        let mut cfg = vec![0x17, 0, 0, 0, 0, 0x01, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4];
        cfg.extend_from_slice(&[0x67, 0x42, 0, 0x1e]);
        cfg.push(1);
        cfg.extend_from_slice(&[0, 4, 0x68, 0xce, 6, 0xe2]);
        let mut out = Vec::new();
        seg.push(&Tag { tag_type: 9, timestamp: 0, data: cfg }, &mut out);

        let mut key = vec![0x17, 1, 0, 0, 0, 0, 0, 0, 4];
        key.extend_from_slice(&[0x65, 1, 2, 3]);
        seg.push(&Tag { tag_type: 9, timestamp: 0, data: key.clone() }, &mut out);
        seg.push(&Tag { tag_type: 9, timestamp: 1500, data: key }, &mut out);
        assert!(!out.is_empty());
        assert_eq!(out[0].data[0], 0x47);
    }
}
