//! MPEG-TS muxer for HLS (H.264 + AAC).

const PAT_PID: u16 = 0x0000;
const PMT_PID: u16 = 0x1000;
const VIDEO_PID: u16 = 0x0100;
const AUDIO_PID: u16 = 0x0101;

pub struct TsMuxer {
    continuity: [u8; 0x2000],
    pat_written: bool,
    pmt_written: bool,
}

impl Default for TsMuxer {
    fn default() -> Self {
        Self::new()
    }
}

impl TsMuxer {
    pub fn new() -> Self {
        Self {
            continuity: [0; 0x2000],
            pat_written: false,
            pmt_written: false,
        }
    }

    pub fn write_tables(&mut self, out: &mut Vec<u8>) {
        if !self.pat_written {
            self.write_psi(out, PAT_PID, pat_section());
            self.pat_written = true;
        }
        if !self.pmt_written {
            self.write_psi(out, PMT_PID, pmt_section());
            self.pmt_written = true;
        }
    }

    pub fn write_video(&mut self, out: &mut Vec<u8>, annex_b: &[u8], pts: u64, dts: u64) {
        self.write_tables(out);
        let pes = pes(0xE0, Some((pts, dts)), annex_b);
        self.write_pes(out, VIDEO_PID, &pes, Some(pts));
    }

    pub fn write_audio(&mut self, out: &mut Vec<u8>, adts: &[u8], pts: u64) {
        self.write_tables(out);
        let pes = pes(0xC0, Some((pts, pts)), adts);
        self.write_pes(out, AUDIO_PID, &pes, None);
    }

    fn write_psi(&mut self, out: &mut Vec<u8>, pid: u16, section: Vec<u8>) {
        let mut payload = vec![0u8];
        payload.extend_from_slice(&section);
        self.write_ts(out, pid, true, &payload, None);
    }

    fn write_pes(&mut self, out: &mut Vec<u8>, pid: u16, pes: &[u8], pcr: Option<u64>) {
        let mut offset = 0;
        let mut first = true;
        while offset < pes.len() {
            let pcr_here = if first { pcr } else { None };
            let header_len = if pcr_here.is_some() { 12 } else { 4 };
            let take = (188 - header_len).min(pes.len() - offset);
            self.write_ts(out, pid, first, &pes[offset..offset + take], pcr_here);
            offset += take;
            first = false;
        }
    }

    fn write_ts(
        &mut self,
        out: &mut Vec<u8>,
        pid: u16,
        start: bool,
        payload: &[u8],
        pcr: Option<u64>,
    ) {
        let mut packet = [0xffu8; 188];
        packet[0] = 0x47;
        packet[1] = ((start as u8) << 6) | ((pid >> 8) as u8 & 0x1f);
        packet[2] = (pid & 0xff) as u8;
        let cc = self.continuity[pid as usize & 0x1fff];
        self.continuity[pid as usize & 0x1fff] = cc.wrapping_add(1);
        let pos;
        if let Some(pcr) = pcr {
            packet[3] = 0x30 | (cc & 0x0f);
            let stuffing = 176usize.saturating_sub(payload.len());
            packet[4] = (7 + stuffing) as u8;
            packet[5] = 0x10;
            write_pcr(&mut packet[6..12], pcr);
            pos = 12 + stuffing;
        } else if payload.len() == 184 {
            packet[3] = 0x10 | (cc & 0x0f);
            pos = 4;
        } else {
            packet[3] = 0x30 | (cc & 0x0f);
            let adapt_len = 183 - payload.len();
            packet[4] = adapt_len as u8;
            packet[5] = 0;
            pos = 5 + adapt_len;
        }
        let n = payload.len().min(188 - pos);
        packet[pos..pos + n].copy_from_slice(&payload[..n]);
        out.extend_from_slice(&packet);
    }
}

fn pat_section() -> Vec<u8> {
    let mut s = Vec::with_capacity(16);
    s.push(0x00);
    s.push(0xb0);
    s.push(13);
    s.extend_from_slice(&1u16.to_be_bytes());
    s.push(0xc1);
    s.push(0);
    s.push(0);
    s.extend_from_slice(&1u16.to_be_bytes());
    s.extend_from_slice(&(0xe000u16 | PMT_PID).to_be_bytes());
    let crc = crc32(&s);
    s.extend_from_slice(&crc.to_be_bytes());
    s
}

fn pmt_section() -> Vec<u8> {
    let mut s = Vec::with_capacity(32);
    s.push(0x02);
    s.push(0xb0);
    s.push(23);
    s.extend_from_slice(&1u16.to_be_bytes());
    s.push(0xc1);
    s.push(0);
    s.push(0);
    s.extend_from_slice(&(0xe000u16 | VIDEO_PID).to_be_bytes());
    s.extend_from_slice(&0xf000u16.to_be_bytes());
    s.push(0x1b);
    s.extend_from_slice(&(0xe000u16 | VIDEO_PID).to_be_bytes());
    s.extend_from_slice(&0xf000u16.to_be_bytes());
    s.push(0x0f);
    s.extend_from_slice(&(0xe000u16 | AUDIO_PID).to_be_bytes());
    s.extend_from_slice(&0xf000u16.to_be_bytes());
    let crc = crc32(&s);
    s.extend_from_slice(&crc.to_be_bytes());
    s
}

fn pes(stream_id: u8, times: Option<(u64, u64)>, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 32);
    out.extend_from_slice(&[0, 0, 1, stream_id]);
    let total = 8 + data.len();
    let len = if total > u16::MAX as usize {
        0
    } else {
        total as u16
    };
    out.extend_from_slice(&len.to_be_bytes());
    out.push(0x80);
    if let Some((pts, dts)) = times {
        if stream_id == 0xE0 && pts != dts {
            out.push(0xc0);
            out.push(10);
            write_ts(&mut out, 0b0011, pts);
            write_ts(&mut out, 0b0001, dts);
        } else {
            out.push(0x80);
            out.push(5);
            write_ts(&mut out, 0b0010, pts);
        }
    } else {
        out.extend_from_slice(&[0, 0]);
    }
    out.extend_from_slice(data);
    out
}

fn write_ts(out: &mut Vec<u8>, prefix: u8, ts: u64) {
    out.push((prefix << 4) | (((ts >> 30) & 0x07) as u8) << 1 | 1);
    out.push((ts >> 22) as u8);
    out.push((((ts >> 15) & 0x7f) as u8) << 1 | 1);
    out.push((ts >> 7) as u8);
    out.push(((ts & 0x7f) as u8) << 1 | 1);
}

fn write_pcr(out: &mut [u8], pcr: u64) {
    let base = pcr & 0x1_ffff_ffff;
    out[0] = (base >> 25) as u8;
    out[1] = (base >> 17) as u8;
    out[2] = (base >> 9) as u8;
    out[3] = (base >> 1) as u8;
    out[4] = ((base & 1) as u8) << 7 | 0x7e;
    out[5] = 0;
}

fn crc32(data: &[u8]) -> u32 {
    // MPEG-2 CRC32: polynomial 0x04C11DB7, init 0xFFFFFFFF, no reflection.
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= (b as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04c1_1db7
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_pat_pmt_and_pes() {
        let mut mux = TsMuxer::new();
        let mut out = Vec::new();
        mux.write_tables(&mut out);
        assert_eq!(out.len(), 376);
        assert_eq!(out[0], 0x47);
        mux.write_video(&mut out, &[0, 0, 0, 1, 0x65, 1, 2, 3], 9000, 9000);
        assert!(out.len() > 376);
    }
}
