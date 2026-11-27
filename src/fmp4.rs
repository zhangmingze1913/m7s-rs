//! On-demand fMP4 pack from the current GOP cache (video-only).

use m7s_codec::FlvTag;
use m7s_plugin_hls::fmp4::{Sample, init_segment, media_segment};

pub struct Part {
    pub data: Vec<u8>,
    pub duration: f64,
}

pub struct Segment {
    pub data: Vec<u8>,
    pub duration: f64,
    pub parts: Vec<Part>,
}

fn avc_config(data: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    if data.len() < 11 || data[0] != 0x17 || data[1] != 0 {
        return None;
    }
    let cfg = &data[5..];
    let sps_len = u16::from_be_bytes([cfg[6], cfg[7]]) as usize;
    if cfg.len() < 8 + sps_len + 3 {
        return None;
    }
    let sps = cfg[8..8 + sps_len].to_vec();
    let pps_len = u16::from_be_bytes([cfg[8 + sps_len + 1], cfg[8 + sps_len + 2]]) as usize;
    if cfg.len() < 8 + sps_len + 3 + pps_len {
        return None;
    }
    let pps = cfg[8 + sps_len + 3..8 + sps_len + 3 + pps_len].to_vec();
    Some((sps, pps))
}

fn build_segment(seq: u32, base: u64, samples: &[Sample]) -> Segment {
    let data = media_segment(seq, base, samples, 90_000);
    let total: u32 = samples.iter().map(|s| s.duration).sum();
    let mut parts = Vec::new();
    let mut start = base;
    let mut first = 0usize;
    let mut dur = 0u32;
    // Slice the sample array instead of cloning every sample into a scratch
    // Vec: `media_segment` only reads, so a sub-slice is enough.
    for (i, s) in samples.iter().enumerate() {
        dur += s.duration;
        if dur >= 45_000 {
            let d = dur as f64 / 90_000.0;
            parts.push(Part {
                data: media_segment(seq, start, &samples[first..=i], 90_000),
                duration: d,
            });
            start += dur as u64;
            dur = 0;
            first = i + 1;
        }
    }
    if first < samples.len() {
        let d = dur as f64 / 90_000.0;
        parts.push(Part {
            data: media_segment(seq, start, &samples[first..], 90_000),
            duration: d,
        });
    }
    Segment { data, duration: total as f64 / 90_000.0, parts }
}
/// Returns (init, all GOP segments).
pub fn pack_all(tags: &[FlvTag]) -> Option<(Vec<u8>, Vec<Segment>)> {
    let config = tags
        .iter()
        .find(|t| t.tag_type == 9 && t.data.len() > 5 && t.data[1] == 0)?;
    let (sps, pps) = avc_config(&config.data)?;
    let mut samples = Vec::new();
    for (i, tag) in tags.iter().enumerate() {
        if tag.tag_type != 9 || tag.data.len() <= 5 || tag.data[1] != 1 {
            continue;
        }
        if tag.data[0] >> 4 != 1 {
            continue;
        }
        let next = tags
            .get(i + 1)
            .filter(|t| t.tag_type == 9)
            .map(|t| t.timestamp)
            .unwrap_or(tag.timestamp + 40);
        let duration = next.saturating_sub(tag.timestamp).max(1) * 90;
        samples.push(Sample {
            data: tag.data[5..].to_vec(),
            duration,
            key: tag.data[0] >> 4 == 1,
        });
    }
    if samples.is_empty() {
        return None;
    }
    let init = init_segment(&sps, &pps, 320, 240, 90_000);
    let mut segs = Vec::new();
    let mut cur: Vec<Sample> = Vec::new();
    let mut base = 0u64;
    let mut seq = 1u32;
    for s in samples {
        if s.key && !cur.is_empty() {
            let total: u32 = cur.iter().map(|x| x.duration).sum();
            segs.push(build_segment(seq, base, &cur));
            base += total as u64;
            seq += 1;
            cur.clear();
        }
        cur.push(s);
    }
    if !cur.is_empty() {
        segs.push(build_segment(seq, base, &cur));
    }
    if segs.is_empty() {
        return None;
    }
    Some((init, segs))
}

/// Returns (init, first media segment, EXTINF seconds).
pub fn pack(tags: &[FlvTag]) -> Option<(Vec<u8>, Vec<u8>, f64)> {
    let (init, segs) = pack_all(tags)?;
    let first = segs.into_iter().next()?;
    Some((init, first.data, first.duration))
}

pub fn playlist(segs: &[Segment]) -> String {
    let mut out = String::from(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-PART-INF:PART-TARGET=0.5\n#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.0\n",
    );
    for (i, s) in segs.iter().enumerate() {
        for (j, part) in s.parts.iter().enumerate() {
            out.push_str(&format!("#EXT-X-PART:DURATION={:.3},URI=\"part-{i}-{j}.m4s\"\n", part.duration));
        }
        out.push_str(&format!("#EXTINF:{:.3},\nseg-{i}.m4s\n", s.duration));
    }
    out.push_str(&format!("#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"part-{}-0.m4s\"\n", segs.len()));
    out
}

pub fn pack_av(tags: &[FlvTag]) -> Option<(Vec<u8>, Vec<u8>, f64)> {
    let vcfg = tags.iter().find(|t| t.tag_type == 9 && t.data.len() > 5 && t.data[1] == 0)?;
    let (sps, pps) = avc_config(&vcfg.data)?;
    let acfg = tags.iter().find(|t| t.tag_type == 8 && t.data.len() > 2 && t.data[1] == 0)?;
    let asc = acfg.data[2..].to_vec();
    let mut vs = Vec::new();
    let mut started = false;
    for (i, tag) in tags.iter().enumerate() {
        if tag.tag_type != 9 || tag.data.len() <= 5 || tag.data[1] != 1 {
            continue;
        }
        if !started {
            if tag.data[0] >> 4 != 1 { continue; }
            started = true;
        }
        let next = tags.get(i + 1).filter(|t| t.tag_type == 9).map(|t| t.timestamp).unwrap_or(tag.timestamp + 40);
        vs.push(Sample { data: tag.data[5..].to_vec(), duration: next.saturating_sub(tag.timestamp).max(1) * 90, key: tag.data[0] >> 4 == 1 });
    }
    let mut aus = Vec::new();
    for tag in tags {
        if tag.tag_type == 8 && tag.data.len() > 2 && tag.data[1] == 1 {
            aus.push(Sample { data: tag.data[2..].to_vec(), duration: 1024, key: true });
        }
    }
    if vs.is_empty() || aus.is_empty() { return None; }
    let init = m7s_plugin_hls::fmp4::init_segment_av(&sps, &pps, 320, 240, &asc);
    let seg = m7s_plugin_hls::fmp4::media_segment_av(1, 0, &vs, 0, &aus);
    let total: u32 = vs.iter().map(|s| s.duration).sum();
    Some((init, seg, total as f64 / 90_000.0))
}
pub type AvSegment = (Vec<u8>, f64);

pub fn pack_av_all(tags: &[FlvTag]) -> Option<(Vec<u8>, Vec<AvSegment>)> {
    let vcfg = tags.iter().find(|t| t.tag_type == 9 && t.data.len() > 5 && t.data[1] == 0)?;
    let (sps, pps) = avc_config(&vcfg.data)?;
    let acfg = tags.iter().find(|t| t.tag_type == 8 && t.data.len() > 2 && t.data[1] == 0)?;
    let asc = acfg.data[2..].to_vec();

    let mut vs: Vec<(u32, Sample)> = Vec::new();
    let mut started = false;
    for (i, tag) in tags.iter().enumerate() {
        if tag.tag_type != 9 || tag.data.len() <= 5 || tag.data[1] != 1 {
            continue;
        }
        if !started {
            if tag.data[0] >> 4 != 1 { continue; }
            started = true;
        }
        let next = tags.get(i + 1).filter(|t| t.tag_type == 9).map(|t| t.timestamp).unwrap_or(tag.timestamp + 40);
        vs.push((tag.timestamp, Sample { data: tag.data[5..].to_vec(), duration: next.saturating_sub(tag.timestamp).max(1) * 90, key: tag.data[0] >> 4 == 1 }));
    }
    let aus: Vec<(u32, Sample)> = tags
        .iter()
        .filter(|t| t.tag_type == 8 && t.data.len() > 2 && t.data[1] == 1)
        .map(|t| (t.timestamp, Sample { data: t.data[2..].to_vec(), duration: 1024, key: true }))
        .collect();
    if vs.is_empty() || aus.is_empty() { return None; }

    let init = m7s_plugin_hls::fmp4::init_segment_av(&sps, &pps, 320, 240, &asc);
    let mut groups: Vec<Vec<Sample>> = Vec::new();
    let mut starts: Vec<u32> = Vec::new();
    let mut cur = Vec::new();
    let mut cur_start = 0u32;
    for (ts, s) in &vs {
        if s.key && !cur.is_empty() {
            groups.push(std::mem::take(&mut cur));
            starts.push(cur_start);
            cur_start = *ts;
        } else if cur.is_empty() {
            cur_start = *ts;
        }
        cur.push(Sample { data: s.data.clone(), duration: s.duration, key: s.key });
    }
    if !cur.is_empty() {
        groups.push(cur);
        starts.push(cur_start);
    }

    let mut segs = Vec::new();
    let mut total_v = 0u64;
    let mut total_a = 0u64;
    for (i, g) in groups.iter().enumerate() {
        let start = starts[i];
        let end = starts.get(i + 1).copied().unwrap_or(u32::MAX);
        let ag: Vec<Sample> = aus
            .iter()
            .filter(|(ts, _)| *ts >= start && *ts < end)
            .map(|(_, s)| Sample { data: s.data.clone(), duration: s.duration, key: true })
            .collect();
        let vdur: u32 = g.iter().map(|s| s.duration).sum();
        segs.push((m7s_plugin_hls::fmp4::media_segment_av(i as u32 + 1, total_v, g, total_a, &ag), vdur as f64 / 90_000.0));
        total_v += vdur as u64;
        total_a += ag.iter().map(|s| s.duration as u64).sum::<u64>();
    }
    Some((init, segs))
}


use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pub struct Packed {
    pub init: Vec<u8>,
    pub segments: Vec<Segment>,
}

type CacheMap = HashMap<String, (u64, Arc<Packed>)>;

static CACHE: OnceLock<Mutex<CacheMap>> = OnceLock::new();

fn cache() -> &'static Mutex<CacheMap> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn stamp(tags: &[FlvTag]) -> u64 {
    let last = tags.last().map(|t| t.timestamp as u64).unwrap_or(0);
    let last_len = tags.last().map(|t| t.data.len() as u64).unwrap_or(0);
    ((tags.len() as u64) << 32) ^ (last << 8) ^ last_len
}

/// Cached `pack_all` keyed by stream and a cheap cache stamp.
pub fn pack_cached(key: &str, tags: &[FlvTag]) -> Option<Arc<Packed>> {
    let s = stamp(tags);
    if let Some((old, p)) = cache().lock().ok()?.get(key) {
        if *old == s {
            return Some(p.clone());
        }
    }
    let (init, segments) = pack_all(tags)?;
    let p = Arc::new(Packed { init, segments });
    cache()
        .lock()
        .ok()?
        .insert(key.to_string(), (s, p.clone()));
    Some(p)
}
pub fn pack_audio_all(tags: &[FlvTag]) -> Option<(Vec<u8>, Vec<AvSegment>)> {
    let acfg = tags
        .iter()
        .find(|t| t.tag_type == 8 && t.data.len() > 2 && t.data[1] == 0)?;
    let asc = acfg.data[2..].to_vec();
    let samples: Vec<m7s_plugin_hls::fmp4::Sample> = tags
        .iter()
        .filter(|t| t.tag_type == 8 && t.data.len() > 2 && t.data[1] == 1)
        .map(|t| m7s_plugin_hls::fmp4::Sample {
            data: t.data[2..].to_vec(),
            duration: 1024,
            key: true,
        })
        .collect();
    if samples.is_empty() {
        return None;
    }
    let init = m7s_plugin_hls::fmp4::init_segment_audio(&asc, 44_100);
    let dur = samples.len() as f64 * 1024.0 / 44_100.0;
    let seg = m7s_plugin_hls::fmp4::media_segment_audio(1, 0, &samples);
    Some((init, vec![(seg, dur)]))
}
#[cfg(test)]
mod tests {
    use super::*;

    fn tags() -> Vec<FlvTag> {
        let cfg = vec![
            0x17, 0, 0, 0, 0, 1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4,
            0x68, 1, 2, 3,
        ];
        vec![
            FlvTag::new(9, 0, cfg),
            FlvTag::new(9, 0, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 0x65, 1, 2, 3]),
            FlvTag::new(9, 40, vec![0x27, 1, 0, 0, 0, 0, 0, 0, 0x41, 4, 5, 6]),
        ]
    }

    #[test]
    fn packs_synthetic_gop() {
        let (init, seg, _) = pack(&tags()).expect("pack");
        assert!(init.windows(4).any(|w| w == b"moov"));
        assert!(seg.windows(4).any(|w| w == b"moof"));
    }

    #[test]
    fn splits_gops() {
        let mut t = tags();
        t.push(FlvTag::new(9, 80, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 0x65, 7, 8]));
        let (_, segs) = pack_all(&t).expect("segs");
        assert_eq!(segs.len(), 2);
        let p = playlist(&segs);
        assert!(p.contains("seg-1.m4s"));
        assert!(p.contains("#EXT-X-PART:"), "{p}");
        assert!(p.contains("#EXT-X-PART-INF"));
        assert!(p.contains("#EXT-X-PRELOAD-HINT:TYPE=PART"));
    }
    #[test]
    fn av_all_has_segments() {
        let cfg = vec![0x17, 0, 0, 0, 0, 1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4, 0x68, 1, 2, 3];
        let acfg = vec![0xaf, 0, 0x12, 0x10];
        let tags = vec![
            FlvTag::new(9, 0, cfg),
            FlvTag::new(8, 0, acfg),
            FlvTag::new(9, 0, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 0x65, 1, 2]),
            FlvTag::new(8, 0, vec![0xaf, 1, 0x21, 0x10, 0x20]),
        ];
        let (init, segs) = pack_av_all(&tags).expect("av");
        assert!(init.windows(4).any(|w| w == b"mp4a"));
        assert_eq!(segs.len(), 1);
    }
}

