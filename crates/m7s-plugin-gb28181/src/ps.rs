//! MPEG-PS muxing and RTP packetization for GB28181 playback.

pub struct PsMuxer {
    seq: u16,
    ssrc: u32,
}

impl Default for PsMuxer {
    fn default() -> Self {
        Self {
            seq: 0,
            ssrc: 0x1234_5678,
        }
    }
}

fn pts_bytes(pts: u32) -> [u8; 5] {
    let p = pts as u64;
    [
        0x21 | ((((p >> 30) & 0x07) << 1) as u8),
        ((p >> 22) & 0xff) as u8,
        (((p >> 15) & 0x7f) << 1) as u8 | (((p >> 14) & 1) as u8),
        ((p >> 7) & 0xff) as u8,
        (((p & 0x7f) << 1) | 1) as u8,
    ]
}

fn pes(stream_id: u8, pts: u32, data: &[u8]) -> Vec<u8> {
    let len = 8 + 3 + data.len();
    let mut out = Vec::with_capacity(6 + len);
    out.extend_from_slice(&[0, 0, 1, stream_id]);
    out.extend_from_slice(&(len as u16).to_be_bytes());
    out.extend_from_slice(&[0x80, 0x80, 0x05]);
    out.extend_from_slice(&pts_bytes(pts));
    out.extend_from_slice(&[0, 0, 1]);
    out.extend_from_slice(data);
    out
}

impl PsMuxer {
    pub fn pack(&self, video_pts: u32, video: &[u8], audio_pts: Option<u32>, audio: Option<&[u8]>) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&[
            0, 0, 1, 0xba, 0x44, 0, 4, 0, 4, 1, 0, 0, 2, 0, 0,
        ]);
        out.extend_from_slice(&[0, 0, 1, 0xbb, 0, 9, 0x80, 0xc0, 0xa0, 5, 0x21, 0, 1, 0, 1]);
        out.extend_from_slice(&pes(0xe0, video_pts, video));
        if let (Some(apts), Some(audio)) = (audio_pts, audio) {
            out.extend_from_slice(&pes(0xc0, apts, audio));
        }
        out
    }

    pub fn rtp(&mut self, timestamp: u32, ps: &[u8]) -> Vec<Vec<u8>> {
        const MTU: usize = 1400;
        let chunks: Vec<&[u8]> = ps.chunks(MTU).collect();
        let mut out = Vec::with_capacity(chunks.len());
        for (i, chunk) in chunks.iter().enumerate() {
            self.seq = self.seq.wrapping_add(1);
            let mut p = Vec::with_capacity(12 + chunk.len());
            p.push(0x80);
            p.push(if i + 1 == chunks.len() { 0xe0 } else { 0x60 });
            p.extend_from_slice(&self.seq.to_be_bytes());
            p.extend_from_slice(&timestamp.to_be_bytes());
            p.extend_from_slice(&self.ssrc.to_be_bytes());
            p.extend_from_slice(chunk);
            out.push(p);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_has_ps_and_pes() {
        let m = PsMuxer::default();
        let ps = m.pack(9000, &[0x65, 1, 2, 3], Some(1024), Some(&[0x21, 4]));
        assert_eq!(&ps[0..4], &[0, 0, 1, 0xba]);
        assert!(ps.windows(4).any(|w| w == [0, 0, 1, 0xe0]));
        assert!(ps.windows(4).any(|w| w == [0, 0, 1, 0xc0]));
    }

    #[test]
    fn rtp_fragments_and_marks_last() {
        let mut m = PsMuxer::default();
        let data = vec![7u8; 3000];
        let packets = m.rtp(90_000, &data);
        assert_eq!(packets.len(), 3);
        assert_eq!(packets[2][1] & 0x80, 0x80);
        let joined: Vec<u8> = packets.iter().flat_map(|p| p[12..].to_vec()).collect();
        assert_eq!(joined, data);
    }
}

pub async fn send_tags(
    socket: &tokio::net::UdpSocket,
    target: std::net::SocketAddr,
    tags: &[m7s_codec::FlvTag],
) -> std::io::Result<usize> {
    let mut mux = PsMuxer::default();
    let mut sent = 0;
    for tag in tags {
        if tag.tag_type == 9 && tag.data.len() > 5 && tag.data[1] == 1 {
            let pts = tag.timestamp.wrapping_mul(90);
            let ps = mux.pack(pts, &tag.data[5..], None, None);
            for p in mux.rtp(pts, &ps) {
                socket.send_to(&p, target).await?;
                sent += 1;
            }
        } else if tag.tag_type == 8 && tag.data.len() > 2 && tag.data[1] == 1 {
            let pts = tag.timestamp.wrapping_mul(90);
            let ps = mux.pack(pts, &[], Some(pts), Some(&tag.data[2..]));
            for p in mux.rtp(pts, &ps) {
                socket.send_to(&p, target).await?;
                sent += 1;
            }
        }
    }
    Ok(sent)
}

#[cfg(test)]
mod send_tests {
    use super::*;

    #[test]
    fn sends_ps_over_udp() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let recv = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
                let target = recv.local_addr().unwrap();
                let send = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
                let tags = vec![m7s_codec::FlvTag::new(9, 0, vec![0x17,1,0,0,0,0,0,0,0x65,1,2])];
                let n = send_tags(&send, target, &tags).await.unwrap();
                assert!(n >= 1);
                let mut buf = [0u8; 2048];
                let (len, _) = tokio::time::timeout(std::time::Duration::from_secs(1), recv.recv_from(&mut buf))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(len > 12);
                assert_eq!(&buf[12..16], &[0, 0, 1, 0xba]);
            });
    }
}

pub async fn play_flv_file(
    path: &std::path::Path,
    socket: &tokio::net::UdpSocket,
    target: std::net::SocketAddr,
) -> anyhow::Result<usize> {
    let data = std::fs::read(path)?;
    let mut reader = m7s_codec::FlvReader::new();
    reader.push(&data);
    let mut sent = 0;
    while let Some(tag) = reader.next_tag()? {
        sent += send_tags(socket, target, std::slice::from_ref(&tag)).await?;
    }
    Ok(sent)
}

#[cfg(test)]
mod file_tests {
    use super::*;

    #[test]
    fn plays_flv_file() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let mut f = m7s_codec::flv_header(true, true).to_vec();
                let data = vec![0x17, 1, 0, 0, 0, 0, 0, 0, 0x65, 1, 2];
                f.push(9);
                f.extend_from_slice(&(data.len() as u32).to_be_bytes()[1..]);
                f.extend_from_slice(&[0, 0, 0]);
                f.push(0);
                f.extend_from_slice(&[0, 0, 1]);
                f.extend_from_slice(&data);
                f.extend_from_slice(&((11 + data.len()) as u32).to_be_bytes());
                let path = std::path::PathBuf::from(format!("{}/../../target/gb-{}.flv", env!("CARGO_MANIFEST_DIR"), std::process::id()));
                std::fs::write(&path, &f).unwrap();
                let recv = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
                let target = recv.local_addr().unwrap();
                let send = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
                let n = play_flv_file(&path, &send, target).await.unwrap();
                assert!(n >= 1);
                let mut buf = [0u8; 2048];
                let (len, _) = tokio::time::timeout(std::time::Duration::from_secs(1), recv.recv_from(&mut buf))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(len > 12);
                let _ = std::fs::remove_file(path);
            });
    }
}

pub async fn play_flv_file_range(
    path: &std::path::Path,
    socket: &tokio::net::UdpSocket,
    target: std::net::SocketAddr,
    start_ms: u32,
    speed: f32,
) -> anyhow::Result<usize> {
    let data = std::fs::read(path)?;
    let mut reader = m7s_codec::FlvReader::new();
    reader.push(&data);
    let mut sent = 0;
    let mut prev: Option<u32> = None;
    let speed = if speed > 0.0 { speed } else { 1.0 };
    while let Some(tag) = reader.next_tag()? {
        if tag.timestamp < start_ms {
            continue;
        }
        if let Some(p) = prev {
            let delta = tag.timestamp.saturating_sub(p) as f32 / speed;
            if delta > 0.0 {
                tokio::time::sleep(std::time::Duration::from_secs_f32(delta / 1000.0)).await;
            }
        }
        prev = Some(tag.timestamp);
        sent += send_tags(socket, target, std::slice::from_ref(&tag)).await?;
    }
    Ok(sent)
}

#[cfg(test)]
mod range_tests {
    use super::*;

    fn flv_with(timestamps: &[u32]) -> Vec<u8> {
        let mut f = m7s_codec::flv_header(true, true).to_vec();
        for ts in timestamps {
            let data = vec![0x17, 1, 0, 0, 0, 0, 0, 0, 0x65, 1, 2];
            f.push(9);
            f.extend_from_slice(&(data.len() as u32).to_be_bytes()[1..]);
            f.extend_from_slice(&ts.to_be_bytes()[1..]);
            f.push((ts >> 24) as u8);
            f.extend_from_slice(&[0, 0, 1]);
            f.extend_from_slice(&data);
            f.extend_from_slice(&((11 + data.len()) as u32).to_be_bytes());
        }
        f
    }

    #[test]
    fn start_filter() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let path = std::path::PathBuf::from(format!(
                    "{}/../../target/gb-range-{}.flv",
                    env!("CARGO_MANIFEST_DIR"),
                    std::process::id()
                ));
                std::fs::write(&path, flv_with(&[0, 1000])).unwrap();
                let recv = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
                let target = recv.local_addr().unwrap();
                let send = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
                let n = play_flv_file_range(&path, &send, target, 500, 10.0)
                    .await
                    .unwrap();
                assert!(n >= 1);
                let _ = std::fs::remove_file(path);
            });
    }
}

pub async fn play_flv_file_controlled(
    path: &std::path::Path,
    socket: &tokio::net::UdpSocket,
    target: std::net::SocketAddr,
    start_ms: u32,
    speed: f32,
    paused: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> anyhow::Result<usize> {
    let data = std::fs::read(path)?;
    let mut reader = m7s_codec::FlvReader::new();
    reader.push(&data);
    let mut sent = 0;
    let mut prev: Option<u32> = None;
    let speed = if speed > 0.0 { speed } else { 1.0 };
    while let Some(tag) = reader.next_tag()? {
        while paused.load(std::sync::atomic::Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        if tag.timestamp < start_ms {
            continue;
        }
        if let Some(p) = prev {
            let delta = tag.timestamp.saturating_sub(p) as f32 / speed;
            if delta > 0.0 {
                tokio::time::sleep(std::time::Duration::from_secs_f32(delta / 1000.0)).await;
            }
        }
        prev = Some(tag.timestamp);
        sent += send_tags(socket, target, std::slice::from_ref(&tag)).await?;
    }
    Ok(sent)
}

#[cfg(test)]
mod pause_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn pause_blocks_send() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let path = std::path::PathBuf::from(format!(
                    "{}/../../target/gb-pause-{}.flv",
                    env!("CARGO_MANIFEST_DIR"),
                    std::process::id()
                ));
                let mut f = m7s_codec::flv_header(true, true).to_vec();
                let data = vec![0x17, 1, 0, 0, 0, 0, 0, 0, 0x65, 1, 2];
                f.push(9);
                f.extend_from_slice(&(data.len() as u32).to_be_bytes()[1..]);
                f.extend_from_slice(&[0, 0, 0]);
                f.push(0);
                f.extend_from_slice(&[0, 0, 1]);
                f.extend_from_slice(&data);
                f.extend_from_slice(&((11 + data.len()) as u32).to_be_bytes());
                std::fs::write(&path, &f).unwrap();

                let recv = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
                let target = recv.local_addr().unwrap();
                let send = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
                let paused = std::sync::Arc::new(AtomicBool::new(true));
                let p2 = paused.clone();
                let path2 = path.clone();
                let task = tokio::spawn(async move {
                    play_flv_file_controlled(&path2, &send, target, 0, 1.0, p2).await
                });
                let mut buf = [0u8; 2048];
                assert!(
                    tokio::time::timeout(std::time::Duration::from_millis(200), recv.recv_from(&mut buf))
                        .await
                        .is_err()
                );
                paused.store(false, Ordering::Relaxed);
                let n = tokio::time::timeout(std::time::Duration::from_secs(1), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                assert!(n >= 1);
                let _ = std::fs::remove_file(path);
            });
    }
}

pub struct PlaybackControl {
    paused: std::sync::atomic::AtomicBool,
    seek_ms: std::sync::atomic::AtomicU32,
    speed_bits: std::sync::atomic::AtomicU32,
    stop: std::sync::atomic::AtomicBool,
}

impl Default for PlaybackControl {
    fn default() -> Self {
        Self {
            paused: std::sync::atomic::AtomicBool::new(false),
            seek_ms: std::sync::atomic::AtomicU32::new(0),
            speed_bits: std::sync::atomic::AtomicU32::new(1.0f32.to_bits()),
            stop: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl PlaybackControl {
    pub fn set_paused(&self, v: bool) { self.paused.store(v, std::sync::atomic::Ordering::Relaxed); }
    pub fn paused(&self) -> bool { self.paused.load(std::sync::atomic::Ordering::Relaxed) }
    pub fn seek(&self, ms: u32) { self.seek_ms.store(ms, std::sync::atomic::Ordering::Relaxed); }
    pub fn set_speed(&self, speed: f32) {
        self.speed_bits.store(if speed > 0.0 { speed } else { 1.0 }.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }
    pub fn speed(&self) -> f32 { f32::from_bits(self.speed_bits.load(std::sync::atomic::Ordering::Relaxed)) }
    pub fn stop(&self) { self.stop.store(true, std::sync::atomic::Ordering::Relaxed); }
    pub fn stopped(&self) -> bool { self.stop.load(std::sync::atomic::Ordering::Relaxed) }
}

/// Playback with runtime pause/seek/speed/stop control.
pub async fn play_flv_file_live(
    path: &std::path::Path,
    socket: &tokio::net::UdpSocket,
    target: std::net::SocketAddr,
    control: std::sync::Arc<PlaybackControl>,
) -> anyhow::Result<usize> {
    let data = std::fs::read(path)?;
    play_flv_bytes_live(data, socket, target, control).await
}

pub async fn play_flv_bytes_live(
    data: Vec<u8>,
    socket: &tokio::net::UdpSocket,
    target: std::net::SocketAddr,
    control: std::sync::Arc<PlaybackControl>,
) -> anyhow::Result<usize> {
    let mut reader = m7s_codec::FlvReader::new();
    reader.push(&data);
    let mut tags = Vec::new();
    while let Some(tag) = reader.next_tag()? {
        tags.push(tag);
    }
    let mut idx = 0usize;
    let mut applied_seek = 0u32;
    let mut prev: Option<u32> = None;
    let mut sent = 0;
    while idx < tags.len() {
        if control.stopped() {
            break;
        }
        while control.paused() && !control.stopped() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let seek = control.seek_ms.load(std::sync::atomic::Ordering::Relaxed);
        if seek != applied_seek {
            applied_seek = seek;
            idx = tags.iter().position(|t| t.timestamp >= seek).unwrap_or(tags.len());
            prev = None;
        }
        if idx >= tags.len() {
            break;
        }
        let tag = &tags[idx];
        if let Some(p) = prev {
            let delta = tag.timestamp.saturating_sub(p) as f32 / control.speed();
            if delta > 0.0 {
                tokio::time::sleep(std::time::Duration::from_secs_f32(delta / 1000.0)).await;
            }
        }
        prev = Some(tag.timestamp);
        sent += send_tags(socket, target, std::slice::from_ref(tag)).await?;
        idx += 1;
    }
    Ok(sent)
}

#[cfg(test)]
mod live_tests {
    use super::*;

    fn flv(ts: &[u32]) -> Vec<u8> {
        let mut f = m7s_codec::flv_header(true, true).to_vec();
        for t in ts {
            let data = vec![0x17, 1, 0, 0, 0, 0, 0, 0, 0x65, 1, 2];
            f.push(9);
            f.extend_from_slice(&(data.len() as u32).to_be_bytes()[1..]);
            f.extend_from_slice(&t.to_be_bytes()[1..]);
            f.push((t >> 24) as u8);
            f.extend_from_slice(&[0, 0, 1]);
            f.extend_from_slice(&data);
            f.extend_from_slice(&((11 + data.len()) as u32).to_be_bytes());
        }
        f
    }

    #[test]
    fn pause_then_resume() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let path = std::path::PathBuf::from(format!("{}/../../target/gb-live-{}.flv", env!("CARGO_MANIFEST_DIR"), std::process::id()));
            std::fs::write(&path, flv(&[0])).unwrap();
            let recv = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let target = recv.local_addr().unwrap();
            let send = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let ctl = std::sync::Arc::new(PlaybackControl::default());
            ctl.set_paused(true);
            let c = ctl.clone();
            let task = tokio::spawn(async move { play_flv_file_live(&path, &send, target, c).await });
            let mut buf = [0u8; 2048];
            let early = tokio::time::timeout(std::time::Duration::from_millis(100), recv.recv_from(&mut buf)).await;
            assert!(early.is_err(), "paused playback should not send");
            ctl.set_paused(false);
            let got = tokio::time::timeout(std::time::Duration::from_secs(2), recv.recv_from(&mut buf)).await.unwrap().unwrap();
            assert!(got.0 > 12);
            let n = task.await.unwrap().unwrap();
            assert!(n >= 1);
        });
    }
}

fn ffmpeg_bin() -> String {
    std::env::var("M7S_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string())
}

/// Remux an MP4 recording to a temporary FLV, then play it with full controls.
pub async fn play_mp4_file_live(
    mp4: &std::path::Path,
    socket: &tokio::net::UdpSocket,
    target: std::net::SocketAddr,
    control: std::sync::Arc<PlaybackControl>,
) -> anyhow::Result<usize> {
    let input = mp4.to_path_buf();
    let ff = ffmpeg_bin();
    let out = tokio::task::spawn_blocking(move || {
        std::process::Command::new(ff)
            .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(&input)
            .args(["-c", "copy", "-f", "flv", "pipe:1"])
            .output()
    })
    .await??;
    if !out.status.success() || out.stdout.is_empty() {
        anyhow::bail!("mp4 remux failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    play_flv_bytes_live(out.stdout, socket, target, control).await
}


#[cfg(test)]
mod mp4_tests {
    use super::*;

    #[test]
    fn mp4_remux_and_play() {
        let ff = std::env::var("M7S_FFMPEG").unwrap_or_else(|_| "node_modules/ffmpeg-static/ffmpeg.exe".to_string());
        if std::process::Command::new(&ff).arg("-version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir();
        let mp4 = dir.join(format!("gb-src-{}.mp4", std::process::id()));
        let out = std::process::Command::new(&ff)
            .args(["-y", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=160x120:rate=10", "-t", "2", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-f", "mp4", "-movflags", "frag_keyframe+empty_moov", "pipe:1"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        std::fs::write(&mp4, &out.stdout).unwrap();
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let recv = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let target = recv.local_addr().unwrap();
            let send = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let ctl = std::sync::Arc::new(PlaybackControl::default());
            ctl.set_speed(10.0);
            let path = mp4.clone();
            let task = tokio::spawn(async move { play_mp4_file_live(&path, &send, target, ctl).await });
            let mut buf = [0u8; 2048];
            let got = tokio::time::timeout(std::time::Duration::from_secs(10), recv.recv_from(&mut buf)).await;
            assert!(got.is_ok(), "no RTP from mp4 playback");
            let _ = task.await;
        });
        let _ = std::fs::remove_file(&mp4);
    }
}
