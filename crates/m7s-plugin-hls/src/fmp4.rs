//! Fragmented MP4 (fMP4) init/media segment writer for H.264 HLS.
//!
//! Produces an `ftyp+moov` init segment and `moof+mdat` media fragments.
//! Audio and LL-HLS parts are not yet covered.

/// One H.264 sample in AVCC form.
pub struct Sample {
    pub data: Vec<u8>,
    pub duration: u32,
    pub key: bool,
}

fn be32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&be32((8 + payload.len()) as u32));
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

fn full_box(kind: &[u8; 4], version: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(4 + payload.len());
    body.push(version);
    body.extend_from_slice(&flags.to_be_bytes()[1..]);
    body.extend_from_slice(payload);
    boxed(kind, &body)
}

fn matrix() -> Vec<u8> {
    let mut v = Vec::new();
    for x in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        v.extend_from_slice(&be32(x));
    }
    v
}

pub fn ftyp() -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(b"iso6");
    p.extend_from_slice(&be32(0));
    p.extend_from_slice(b"iso6mp41");
    boxed(b"ftyp", &p)
}

fn avcc(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut p = vec![1, sps.get(1).copied().unwrap_or(0x42), sps.get(2).copied().unwrap_or(0), sps.get(3).copied().unwrap_or(0x1e), 0xff];
    p.push(0xe1);
    p.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    p.extend_from_slice(sps);
    p.push(1);
    p.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    p.extend_from_slice(pps);
    boxed(b"avcC", &p)
}

pub fn init_segment(sps: &[u8], pps: &[u8], width: u16, height: u16, timescale: u32) -> Vec<u8> {
    let mut out = ftyp();
    let mut moov = Vec::new();

    let mut mvhd = Vec::new();
    mvhd.extend_from_slice(&be32(0));
    mvhd.extend_from_slice(&be32(0));
    mvhd.extend_from_slice(&be32(timescale));
    mvhd.extend_from_slice(&be32(0));
    mvhd.extend_from_slice(&be32(0x0001_0000));
    mvhd.extend_from_slice(&[0x01, 0x00]);
    mvhd.extend_from_slice(&[0; 10]);
    mvhd.extend_from_slice(&matrix());
    mvhd.extend_from_slice(&[0; 24]);
    mvhd.extend_from_slice(&be32(2));
    moov.extend_from_slice(&full_box(b"mvhd", 0, 0, &mvhd));

    let mut tkhd = Vec::new();
    tkhd.extend_from_slice(&be32(0));
    tkhd.extend_from_slice(&be32(0));
    tkhd.extend_from_slice(&be32(1));
    tkhd.extend_from_slice(&be32(0));
    tkhd.extend_from_slice(&be32(0));
    tkhd.extend_from_slice(&[0; 8]);
    tkhd.extend_from_slice(&[0; 2]);
    tkhd.extend_from_slice(&[0; 2]);
    tkhd.extend_from_slice(&[0; 2]);
    tkhd.extend_from_slice(&[0; 2]);
    tkhd.extend_from_slice(&matrix());
    tkhd.extend_from_slice(&be32((width as u32) << 16));
    tkhd.extend_from_slice(&be32((height as u32) << 16));
    let tkhd_box = full_box(b"tkhd", 0, 3, &tkhd);

    let mut mdhd = Vec::new();
    mdhd.extend_from_slice(&be32(0));
    mdhd.extend_from_slice(&be32(0));
    mdhd.extend_from_slice(&be32(timescale));
    mdhd.extend_from_slice(&be32(0));
    mdhd.extend_from_slice(&[0x55, 0xc4, 0, 0]);
    let mdhd_box = full_box(b"mdhd", 0, 0, &mdhd);

    let mut hdlr = Vec::new();
    hdlr.extend_from_slice(&be32(0));
    hdlr.extend_from_slice(b"vide");
    hdlr.extend_from_slice(&[0; 12]);
    hdlr.extend_from_slice(b"VideoHandler\0");
    let hdlr_box = full_box(b"hdlr", 0, 0, &hdlr);

    let vmhd = full_box(b"vmhd", 0, 1, &[0; 8]);
    let url = full_box(b"url ", 0, 1, &[]);
    let dref = full_box(b"dref", 0, 0, &{
        let mut p = Vec::new();
        p.extend_from_slice(&be32(1));
        p.extend_from_slice(&url);
        p
    });
    let dinf = boxed(b"dinf", &dref);

    let mut avc1 = Vec::new();
    avc1.extend_from_slice(&[0; 6]);
    avc1.extend_from_slice(&1u16.to_be_bytes());
    avc1.extend_from_slice(&[0; 16]);
    avc1.extend_from_slice(&width.to_be_bytes());
    avc1.extend_from_slice(&height.to_be_bytes());
    avc1.extend_from_slice(&be32(0x0048_0000));
    avc1.extend_from_slice(&be32(0x0048_0000));
    avc1.extend_from_slice(&be32(0));
    avc1.extend_from_slice(&1u16.to_be_bytes());
    avc1.extend_from_slice(&[0; 32]);
    avc1.extend_from_slice(&0x0018u16.to_be_bytes());
    avc1.extend_from_slice(&0xffffu16.to_be_bytes());
    avc1.extend_from_slice(&avcc(sps, pps));
    let stsd = full_box(b"stsd", 0, 0, &{
        let mut p = Vec::new();
        p.extend_from_slice(&be32(1));
        p.extend_from_slice(&boxed(b"avc1", &avc1));
        p
    });

    let mut stts = Vec::new();
    stts.extend_from_slice(&be32(0));
    let mut stsc = Vec::new();
    stsc.extend_from_slice(&be32(0));
    let mut stsz = Vec::new();
    stsz.extend_from_slice(&be32(0));
    stsz.extend_from_slice(&be32(0));
    let mut stco = Vec::new();
    stco.extend_from_slice(&be32(0));

    let mut stbl = Vec::new();
    stbl.extend_from_slice(&stsd);
    stbl.extend_from_slice(&full_box(b"stts", 0, 0, &stts));
    stbl.extend_from_slice(&full_box(b"stsc", 0, 0, &stsc));
    stbl.extend_from_slice(&full_box(b"stsz", 0, 0, &stsz));
    stbl.extend_from_slice(&full_box(b"stco", 0, 0, &stco));
    let stbl = boxed(b"stbl", &stbl);

    let mut minf = Vec::new();
    minf.extend_from_slice(&vmhd);
    minf.extend_from_slice(&dinf);
    minf.extend_from_slice(&stbl);
    let minf = boxed(b"minf", &minf);

    let mut mdia = Vec::new();
    mdia.extend_from_slice(&mdhd_box);
    mdia.extend_from_slice(&hdlr_box);
    mdia.extend_from_slice(&minf);
    let mdia = boxed(b"mdia", &mdia);

    let mut trak = Vec::new();
    trak.extend_from_slice(&tkhd_box);
    trak.extend_from_slice(&mdia);
    moov.extend_from_slice(&boxed(b"trak", &trak));

    let mut trex = Vec::new();
    trex.extend_from_slice(&be32(1));
    trex.extend_from_slice(&be32(1));
    trex.extend_from_slice(&be32(0));
    trex.extend_from_slice(&be32(0));
    trex.extend_from_slice(&be32(0));
    moov.extend_from_slice(&boxed(b"mvex", &full_box(b"trex", 0, 0, &trex)));

    out.extend_from_slice(&boxed(b"moov", &moov));
    out
}

pub fn media_segment(sequence: u32, base_time: u64, samples: &[Sample], timescale: u32) -> Vec<u8> {
    let mut trun = Vec::new();
    trun.extend_from_slice(&be32(samples.len() as u32));
    trun.extend_from_slice(&be32(0));
    for s in samples {
        trun.extend_from_slice(&be32(s.duration));
        trun.extend_from_slice(&be32(s.data.len() as u32));
        trun.extend_from_slice(&be32(if s.key { 0x0200_0000 } else { 0x0101_0000 }));
    }
    let trun = full_box(b"trun", 0, 0x000701, &trun);

    let tfdt = full_box(b"tfdt", 1, 0, &{
        let mut p = Vec::new();
        p.extend_from_slice(&base_time.to_be_bytes());
        p
    });
    let tfhd = full_box(b"tfhd", 0, 0x020000, &be32(1));
    let mut traf = Vec::new();
    traf.extend_from_slice(&tfhd);
    traf.extend_from_slice(&tfdt);
    traf.extend_from_slice(&trun);
    let traf = boxed(b"traf", &traf);

    let mut moof = Vec::new();
    moof.extend_from_slice(&full_box(b"mfhd", 0, 0, &be32(sequence)));
    moof.extend_from_slice(&traf);
    let mut moof = boxed(b"moof", &moof);
    let trun_start = moof.len() - trun.len();
    let data_offset = (moof.len() + 8) as u32;
    moof[trun_start + 16..trun_start + 20].copy_from_slice(&data_offset.to_be_bytes());

    let mut mdat = Vec::new();
    for s in samples {
        mdat.extend_from_slice(&s.data);
    }
    let mdat = boxed(b"mdat", &mdat);

    let mut out = moof;
    out.extend_from_slice(&mdat);
    let _ = timescale;
    out
}

fn desc(tag: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![tag, payload.len() as u8];
    out.extend_from_slice(payload);
    out
}

fn esds(asc: &[u8]) -> Vec<u8> {
    let dsi = desc(0x05, asc);
    let mut dec = Vec::new();
    dec.extend_from_slice(&[0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    dec.extend_from_slice(&dsi);
    let dec = desc(0x04, &dec);
    let sl = desc(0x06, &[0x02]);
    let mut es = Vec::new();
    es.extend_from_slice(&[0, 1, 0]);
    es.extend_from_slice(&dec);
    es.extend_from_slice(&sl);
    boxed(b"esds", &desc(0x03, &es))
}

fn audio_trak(asc: &[u8], timescale: u32) -> Vec<u8> {
    let mut tkhd = Vec::new();
    tkhd.extend_from_slice(&be32(0));
    tkhd.extend_from_slice(&be32(0));
    tkhd.extend_from_slice(&be32(2));
    tkhd.extend_from_slice(&be32(0));
    tkhd.extend_from_slice(&be32(0));
    tkhd.extend_from_slice(&[0; 8]);
    tkhd.extend_from_slice(&[0; 2]);
    tkhd.extend_from_slice(&[0; 2]);
    tkhd.extend_from_slice(&[0x01, 0x00]);
    tkhd.extend_from_slice(&[0; 2]);
    tkhd.extend_from_slice(&matrix());
    tkhd.extend_from_slice(&be32(0));
    tkhd.extend_from_slice(&be32(0));

    let mut mdhd = Vec::new();
    mdhd.extend_from_slice(&be32(0));
    mdhd.extend_from_slice(&be32(0));
    mdhd.extend_from_slice(&be32(timescale));
    mdhd.extend_from_slice(&be32(0));
    mdhd.extend_from_slice(&[0x55, 0xc4, 0, 0]);

    let mut hdlr = Vec::new();
    hdlr.extend_from_slice(&be32(0));
    hdlr.extend_from_slice(b"soun");
    hdlr.extend_from_slice(&[0; 12]);
    hdlr.extend_from_slice(b"SoundHandler\0");

    let mut mp4a = Vec::new();
    mp4a.extend_from_slice(&[0; 6]);
    mp4a.extend_from_slice(&1u16.to_be_bytes());
    mp4a.extend_from_slice(&[0; 8]);
    mp4a.extend_from_slice(&2u16.to_be_bytes());
    mp4a.extend_from_slice(&16u16.to_be_bytes());
    mp4a.extend_from_slice(&0u32.to_be_bytes());
    mp4a.extend_from_slice(&0u32.to_be_bytes());
    mp4a.extend_from_slice(&(timescale << 16).to_be_bytes());
    mp4a.extend_from_slice(&esds(asc));

    let stsd = full_box(b"stsd", 0, 0, &{
        let mut p = Vec::new();
        p.extend_from_slice(&be32(1));
        p.extend_from_slice(&boxed(b"mp4a", &mp4a));
        p
    });
    let mut stbl = Vec::new();
    stbl.extend_from_slice(&stsd);
    stbl.extend_from_slice(&full_box(b"stts", 0, 0, &be32(0)));
    stbl.extend_from_slice(&full_box(b"stsc", 0, 0, &be32(0)));
    stbl.extend_from_slice(&full_box(b"stsz", 0, 0, &[0, 0, 0, 0, 0, 0, 0, 0]));
    stbl.extend_from_slice(&full_box(b"stco", 0, 0, &be32(0)));

    let mut minf = Vec::new();
    minf.extend_from_slice(&full_box(b"smhd", 0, 0, &[0; 4]));
    let dref = full_box(b"dref", 0, 0, &{
        let mut p = Vec::new();
        p.extend_from_slice(&be32(1));
        p.extend_from_slice(&full_box(b"url ", 0, 1, &[]));
        p
    });
    minf.extend_from_slice(&boxed(b"dinf", &dref));
    minf.extend_from_slice(&boxed(b"stbl", &stbl));

    let mut mdia = Vec::new();
    mdia.extend_from_slice(&full_box(b"mdhd", 0, 0, &mdhd));
    mdia.extend_from_slice(&full_box(b"hdlr", 0, 0, &hdlr));
    mdia.extend_from_slice(&boxed(b"minf", &minf));

    let mut trak = Vec::new();
    trak.extend_from_slice(&full_box(b"tkhd", 0, 3, &tkhd));
    trak.extend_from_slice(&boxed(b"mdia", &mdia));
    boxed(b"trak", &trak)
}

pub fn init_segment_av(sps: &[u8], pps: &[u8], width: u16, height: u16, asc: &[u8]) -> Vec<u8> {
    let base = init_segment(sps, pps, width, height, 90_000);
    let ftyp_size = u32::from_be_bytes([base[0], base[1], base[2], base[3]]) as usize;
    let moov = &base[ftyp_size..];
    let payload = &moov[8..];
    let pos = payload.windows(4).position(|w| w == b"mvex").unwrap_or(payload.len());
    let mut body = Vec::new();
    body.extend_from_slice(&payload[..pos - 4]);
    body.extend_from_slice(&audio_trak(asc, 44_100));
    body.extend_from_slice(&payload[pos - 4..]);
    body.extend_from_slice(&full_box(
        b"trex",
        0,
        0,
        &{
            let mut t = Vec::new();
            t.extend_from_slice(&be32(2));
            t.extend_from_slice(&be32(1));
            t.extend_from_slice(&be32(1024));
            t.extend_from_slice(&be32(0));
            t.extend_from_slice(&be32(0));
            t
        },
    ));
    let mut out = base[..ftyp_size].to_vec();
    out.extend_from_slice(&boxed(b"moov", &body));
    out
}

pub fn media_segment_av(
    sequence: u32,
    vbase: u64,
    video: &[Sample],
    abase: u64,
    audio: &[Sample],
) -> Vec<u8> {
    let mut trun_v = Vec::new();
    trun_v.extend_from_slice(&be32(video.len() as u32));
    trun_v.extend_from_slice(&be32(0));
    for s in video {
        trun_v.extend_from_slice(&be32(s.duration));
        trun_v.extend_from_slice(&be32(s.data.len() as u32));
        trun_v.extend_from_slice(&be32(if s.key { 0x0200_0000 } else { 0x0101_0000 }));
    }
    let trun_v = full_box(b"trun", 0, 0x000701, &trun_v);
    let mut trun_a = Vec::new();
    trun_a.extend_from_slice(&be32(audio.len() as u32));
    trun_a.extend_from_slice(&be32(0));
    for s in audio {
        trun_a.extend_from_slice(&be32(s.duration));
        trun_a.extend_from_slice(&be32(s.data.len() as u32));
        trun_a.extend_from_slice(&be32(0x0200_0000));
    }
    let trun_a = full_box(b"trun", 0, 0x000701, &trun_a);

    let tfhd = |id: u32| {
        let mut t = Vec::new();
        t.extend_from_slice(&be32(id));
        full_box(b"tfhd", 0, 0x020000, &t)
    };
    let tfdt = |base: u64| full_box(b"tfdt", 1, 0, &base.to_be_bytes());
    let traf = |id: u32, base: u64, trun: &[u8]| {
        let mut t = Vec::new();
        t.extend_from_slice(&tfhd(id));
        t.extend_from_slice(&tfdt(base));
        t.extend_from_slice(trun);
        boxed(b"traf", &t)
    };
    let tv = traf(1, vbase, &trun_v);
    let ta = traf(2, abase, &trun_a);

    let mut moof = Vec::new();
    moof.extend_from_slice(&full_box(b"mfhd", 0, 0, &be32(sequence)));
    moof.extend_from_slice(&tv);
    moof.extend_from_slice(&ta);
    let mut moof = boxed(b"moof", &moof);
    let video_len: usize = video.iter().map(|s| s.data.len()).sum();
    let v_off = (moof.len() + 8) as u32;
    let a_off = v_off + video_len as u32;
    let find = |start: usize, end: usize| {
        (start..end).find(|&i| &moof[i..i + 4] == b"trun").unwrap_or(start)
    };
    let tv_start = 8 + 16;
    let v_trun = find(tv_start, tv_start + tv.len());
    let ta_start = tv_start + tv.len();
    let a_trun = find(ta_start, ta_start + ta.len());
    moof[v_trun + 12..v_trun + 16].copy_from_slice(&v_off.to_be_bytes());
    moof[a_trun + 12..a_trun + 16].copy_from_slice(&a_off.to_be_bytes());

    let mut mdat = Vec::new();
    for s in video {
        mdat.extend_from_slice(&s.data);
    }
    for s in audio {
        mdat.extend_from_slice(&s.data);
    }
    let mut out = moof;
    out.extend_from_slice(&boxed(b"mdat", &mdat));
    out
}
#[cfg(test)]
mod tests {
    use super::*;

    fn kind(data: &[u8], pos: usize) -> &[u8] {
        &data[pos + 4..pos + 8]
    }

    #[test]
    fn init_boxes() {
        let sps = [0x67, 0x42, 0x00, 0x1e];
        let pps = [0x68, 0xce, 0x06, 0xe2];
        let init = init_segment(&sps, &pps, 320, 240, 90_000);
        assert_eq!(kind(&init, 0), b"ftyp");
        let moov = 8 + 8 + init[8..12].len();
        let _ = moov;
        assert!(init.windows(4).any(|w| w == b"moov"));
        assert!(init.windows(4).any(|w| w == b"avcC"));
        assert!(init.windows(4).any(|w| w == b"stbl"));
    }

    #[test]
    fn init_av_has_audio() {
        let sps = [0x67, 0x42, 0x00, 0x1e];
        let pps = [0x68, 0xce, 0x06, 0xe2];
        let init = init_segment_av(&sps, &pps, 320, 240, &[0x12, 0x10]);
        assert_eq!(init.windows(4).filter(|w| *w == b"trak").count(), 2);
        assert!(init.windows(4).any(|w| w == b"mp4a"));
        assert!(init.windows(4).any(|w| w == b"esds"));
        let trex: Vec<usize> = init.windows(4).enumerate().filter(|(_, w)| *w == b"trex").map(|(i, _)| i).collect();
        assert_eq!(trex.len(), 2);
        for i in trex {
            let at = i - 4;
            assert_eq!(u32::from_be_bytes([init[at], init[at + 1], init[at + 2], init[at + 3]]), 32);
        }
    }

    #[test]
    fn fragment_boxes() {
        let seg = media_segment(
            1,
            0,
            &[Sample {
                data: vec![0, 0, 0, 2, 0x65, 0x88],
                duration: 3600,
                key: true,
            }],
            90_000,
        );
        assert_eq!(kind(&seg, 0), b"moof");
        assert!(seg.windows(4).any(|w| w == b"trun"));
        assert!(seg.windows(4).any(|w| w == b"mdat"));
    }
}


pub fn init_segment_audio(asc: &[u8], timescale: u32) -> Vec<u8> {
    let mut out = ftyp();
    let mut moov = Vec::new();
    let mut mvhd = Vec::new();
    mvhd.extend_from_slice(&be32(0));
    mvhd.extend_from_slice(&be32(0));
    mvhd.extend_from_slice(&be32(timescale));
    mvhd.extend_from_slice(&be32(0));
    mvhd.extend_from_slice(&be32(0x0001_0000));
    mvhd.extend_from_slice(&[0x01, 0x00]);
    mvhd.extend_from_slice(&[0; 10]);
    mvhd.extend_from_slice(&matrix());
    mvhd.extend_from_slice(&[0; 24]);
    mvhd.extend_from_slice(&be32(1));
    moov.extend_from_slice(&full_box(b"mvhd", 0, 0, &mvhd));
    moov.extend_from_slice(&audio_trak(asc, timescale));
    let mut trex = Vec::new();
    trex.extend_from_slice(&be32(1));
    trex.extend_from_slice(&be32(1));
    trex.extend_from_slice(&be32(1024));
    trex.extend_from_slice(&be32(0));
    trex.extend_from_slice(&be32(0));
    let mvex = boxed(b"mvex", &full_box(b"trex", 0, 0, &trex));
    moov.extend_from_slice(&mvex);
    out.extend_from_slice(&boxed(b"moov", &moov));
    out
}

pub fn media_segment_audio(sequence: u32, base: u64, samples: &[Sample]) -> Vec<u8> {
    let mut trun = Vec::new();
    trun.extend_from_slice(&be32(samples.len() as u32));
    trun.extend_from_slice(&be32(0));
    for s in samples {
        trun.extend_from_slice(&be32(s.duration));
        trun.extend_from_slice(&be32(s.data.len() as u32));
        trun.extend_from_slice(&be32(0x0200_0000));
    }
    let trun = full_box(b"trun", 0, 0x000701, &trun);
    let mut tfhd = Vec::new();
    tfhd.extend_from_slice(&be32(1));
    let tfhd = full_box(b"tfhd", 0, 0x020000, &tfhd);
    let tfdt = full_box(b"tfdt", 1, 0, &base.to_be_bytes());
    let mut traf = Vec::new();
    traf.extend_from_slice(&tfhd);
    traf.extend_from_slice(&tfdt);
    traf.extend_from_slice(&trun);
    let traf = boxed(b"traf", &traf);
    let mut moof = Vec::new();
    moof.extend_from_slice(&full_box(b"mfhd", 0, 0, &be32(sequence)));
    moof.extend_from_slice(&traf);
    let mut moof = boxed(b"moof", &moof);
    let data_offset = (moof.len() + 8) as u32;
    if let Some(pos) = moof.windows(4).position(|w| w == b"trun") {
        moof[pos + 12..pos + 16].copy_from_slice(&data_offset.to_be_bytes());
    }
    let mut mdat = Vec::new();
    for s in samples {
        mdat.extend_from_slice(&s.data);
    }
    let mut out = moof;
    out.extend_from_slice(&boxed(b"mdat", &mdat));
    out
}

#[cfg(test)]
mod audio_only_tests {
    use super::*;

    #[test]
    fn audio_boxes() {
        let init = init_segment_audio(&[0x12, 0x10], 44_100);
        assert!(init.windows(4).any(|w| w == b"ftyp"));
        assert!(init.windows(4).any(|w| w == b"moov"));
        assert!(init.windows(4).any(|w| w == b"mp4a"));
        assert!(init.windows(4).any(|w| w == b"trex"));
        let samples = vec![
            Sample { data: vec![1, 2, 3], duration: 1024, key: true },
            Sample { data: vec![4, 5], duration: 1024, key: true },
        ];
        let seg = media_segment_audio(1, 0, &samples);
        assert!(seg.windows(4).any(|w| w == b"moof"));
        assert!(seg.windows(4).any(|w| w == b"mdat"));
    }
}
