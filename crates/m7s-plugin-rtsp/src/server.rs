//! RTSP plugin (TCP interleaved): OPTIONS/DESCRIBE/SETUP/PLAY/TEARDOWN.

//!

//! Video is H.264 from the shared stream; RTP is sent over the RTSP TCP

//! connection (interleaved channel 0). This is the first real protocol plugin

//! after the Phase 1 media core, not a stub.

use std::net::SocketAddr;

use std::sync::Arc;

use anyhow::{Context, Result, bail};



use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt, BufWriter, ReadHalf, split};

use tokio::net::{TcpListener, TcpStream, UdpSocket};


fn multicast_interface() -> std::net::Ipv4Addr {
    if let Ok(v) = std::env::var("M7S_RTSP_MULTICAST_IF") {
        if let Ok(ip) = v.parse() {
            return ip;
        }
    }
    if let Ok(s) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if s.connect("8.8.8.8:80").is_ok() {
            if let Ok(std::net::SocketAddr::V4(a)) = s.local_addr() {
                return *a.ip();
            }
        }
    }
    std::net::Ipv4Addr::UNSPECIFIED
}

static MULTICAST_SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
use m7s_codec::FlvTag;

use m7s_sdk::{StreamApi, TagStream};
use super::{rtcp_sr, sdp};

pub type SharedTag = Arc<FlvTag>;

#[derive(Clone)]
pub struct RtspAuth {
    pub user: String,
    pub password: String,
}

pub async fn run(listener: TcpListener, streams: Arc<dyn StreamApi>) -> Result<()> {
    let auth = std::env::var("M7S_RTSP_PASSWORD").ok().filter(|p| !p.is_empty()).map(|password| RtspAuth {
        user: std::env::var("M7S_RTSP_USER").unwrap_or_else(|_| "admin".to_string()),
        password,
    });
    run_with_auth(listener, streams, auth).await
}

pub async fn run_with_context(listener: TcpListener, ctx: m7s_sdk::EngineContext) -> Result<()> {
    let streams = ctx.stream_api().ok_or_else(|| anyhow::anyhow!("EngineContext missing streams"))?;
    run(listener, streams).await
}

pub async fn run_with_auth(
    listener: TcpListener,
    streams: Arc<dyn StreamApi>,
    auth: Option<RtspAuth>,
) -> Result<()> {

    log::info!("RTSP listening on {}", listener.local_addr()?);
    loop {
        let (socket, peer) = listener.accept().await?;
        let Some(guard) = m7s_sdk::connections::try_acquire() else {
            log::warn!("RTSP connection limit reached");
            drop(socket);
            continue;
        };
        if !m7s_sdk::plugins::enabled("rtsp") {
            drop(socket);
            continue;
        }
        socket.set_nodelay(true).ok();
        let streams = streams.clone();
        let auth = auth.clone();
        tokio::spawn(async move {
            let _guard = guard;
            if let Err(e) = handle(socket, streams, auth).await {
                log::debug!("RTSP client {peer}: {e:#}");
            }
        });
    }
}

struct UdpSetup {
    rtp: Arc<UdpSocket>,

    #[allow(dead_code)]
    rtcp: Arc<UdpSocket>,

    #[allow(dead_code)]
    client_rtcp: SocketAddr,

    _rtcp: Arc<UdpSocket>,

    client_rtp: SocketAddr,
}

fn client_ports(headers: &str) -> Option<(u16, u16)> {
    super::client_ports(headers)
}

async fn handle(mut socket: TcpStream, streams: Arc<dyn StreamApi>, auth: Option<RtspAuth>) -> Result<()> {
    let mut buf = Vec::new();

    let mut video_udp: Option<UdpSetup> = None;

    let mut audio_udp: Option<UdpSetup> = None;

    let mut video_setup = false;

    let mut audio_setup = false;

    loop {
        let request = read_request(&mut socket, &mut buf).await?;

        let method = request.method.clone();

        log::info!("RTSP {} cseq={:?}", request.method, request.cseq);
        if request.method != "OPTIONS" {
            if let Some(cfg) = &auth {
                if !authorized(&request, cfg) {
                    send_unauthorized(&mut socket, &request).await?;
                    continue;
                }
            }
        }

        let (app, name) = parse_path(&request.url).unwrap_or(("live", "test"));

        match method.as_str() {
            "OPTIONS" => {
                respond(
                    &mut socket,
                    &request,
                    200,
                    "Public: OPTIONS, DESCRIBE, SETUP, PLAY, TEARDOWN\r\n\r\n",
                )
                .await?;
            }

            "DESCRIBE" => {
                let Some(stream) = streams.stream(app, name) else {
                    respond(&mut socket, &request, 404, "").await?;

                    continue;
                };

                let cache: Vec<SharedTag> = stream.snapshot().into_iter().map(|t| Arc::new(FlvTag::new(t.tag_type, t.timestamp, t.data))).collect();

                let (sps, pps) = avc_config(&cache);

                let asc = audio_config(&cache);

                stream.leave();

                let sdp = sdp(&sps, &pps, &asc);

                respond(
                    &mut socket,
                    &request,
                    200,
                    &format!(
                        "Content-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{}",
                        sdp.len(),
                        sdp
                    ),
                )
                .await?;
            }

            "SETUP" => {
                let track = track_id(&request.url);
                let multicast = request.headers.to_ascii_lowercase().contains("multicast");
                log::info!("SETUP track={track} multicast={multicast} udp={}", client_ports(&request.headers).is_some());
                if multicast {
                    let n = MULTICAST_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let group = std::net::Ipv4Addr::new(239, 255, 1, (n % 250 + 1) as u8);
                    let rtp_port = 30000 + ((n * 2) % 20000) as u16;
                    let rtcp_port = rtp_port + 1;
                    let iface = multicast_interface();
                    let sock2 = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))?;
                    sock2.set_reuse_address(true)?;
                    sock2.bind(&SocketAddr::from(([0, 0, 0, 0], 0)).into())?;
                    sock2.set_multicast_if_v4(&iface)?;
                    sock2.set_multicast_loop_v4(true)?;
                    sock2.set_multicast_ttl_v4(16)?;
                    sock2.set_nonblocking(true)?;
                    let std_sock: std::net::UdpSocket = sock2.into();
                    let rtp = Arc::new(UdpSocket::from_std(std_sock)?);
                    let rtcp = Arc::new(UdpSocket::bind("0.0.0.0:0").await?);

                    let setup = UdpSetup {
                        rtp,
                        rtcp: rtcp.clone(),
                        client_rtcp: SocketAddr::new(std::net::IpAddr::V4(group), rtcp_port),
                        _rtcp: rtcp,
                        client_rtp: SocketAddr::new(std::net::IpAddr::V4(group), rtp_port),
                    };
                    if track == 1 { audio_setup = true; audio_udp = Some(setup); }
                    else { video_setup = true; video_udp = Some(setup); }
                    respond(&mut socket, &request, 200, &format!(
                        "Transport: RTP/AVP;multicast;destination={group};port={rtp_port}-{rtcp_port};ttl=16;ssrc=12345678\r\nSession: m7s;timeout=60\r\n\r\n"
                    )).await?;
                } else if let Some((cp1, cp2)) = client_ports(&request.headers) {
                    let peer = socket.peer_addr()?;

                    let rtp = Arc::new(UdpSocket::bind("0.0.0.0:0").await?);

                    let rtcp = Arc::new(UdpSocket::bind("0.0.0.0:0").await?);

                    let sp1 = rtp.local_addr()?.port();

                    let sp2 = rtcp.local_addr()?.port();

                    let setup = UdpSetup {
                        rtp,

                        rtcp: rtcp.clone(),

                        client_rtcp: SocketAddr::new(peer.ip(), cp2),

                        _rtcp: rtcp,

                        client_rtp: SocketAddr::new(peer.ip(), cp1),
                    };

                    if track == 1 {
                        audio_setup = true;

                        audio_udp = Some(setup);
                    } else {
                        video_setup = true;

                        video_udp = Some(setup);
                    }

                    respond(&mut socket, &request, 200, &format!("Transport: RTP/AVP;unicast;client_port={cp1}-{cp2};server_port={sp1}-{sp2};ssrc=12345678\r\nSession: m7s;timeout=60\r\n\r\n")).await?;
                } else {
                    if track == 1 {
                        audio_setup = true;
                    } else {
                        video_setup = true;
                    }

                    respond(&mut socket, &request, 200, if track == 1 { "Transport: RTP/AVP/TCP;unicast;interleaved=2-3\r\nSession: m7s;timeout=60\r\n\r\n" } else { "Transport: RTP/AVP/TCP;unicast;interleaved=0-1\r\nSession: m7s;timeout=60\r\n\r\n" }).await?;
                }
            }

            "PLAY" => {
                let Some(stream) = streams.stream(app, name) else {
                    respond(&mut socket, &request, 404, "").await?;

                    continue;
                };

                let cache: Vec<SharedTag> = stream.snapshot().into_iter().map(|t| Arc::new(FlvTag::new(t.tag_type, t.timestamp, t.data))).collect();

                let vts = cache
                    .iter()
                    .find(|t| t.tag_type == 9)
                    .map(|t| t.timestamp.wrapping_mul(90))
                    .unwrap_or(0);

                let ats = cache
                    .iter()
                    .find(|t| t.tag_type == 8)
                    .map(|t| t.timestamp.wrapping_mul(90))
                    .unwrap_or(0);

                respond(



                    &mut socket,



                    &request,



                    200,



                    &format!(



                        "Session: m7s;timeout=60\r\nRange: npt=0.000-\r\nRTP-Info: url={}/trackID=0;seq=0;rtptime={vts},url={}/trackID=1;seq=0;rtptime={ats}\r\n\r\n",



                        request.url, request.url



                    ),



                )



                .await?;

                if video_udp.is_some() || audio_udp.is_some() {
                    let reader = stream.tag_reader();
                    tokio::spawn(play_udp(
                        stream,
                        socket,
                        video_udp.take(),
                        audio_udp.take(),
                        reader,
                        video_setup,
                        audio_setup,
                    ));
                } else {
                    let reader = stream.tag_reader();
                    tokio::spawn(play_loop(
                        stream,
                        socket,
                        reader,
                        video_setup,
                        audio_setup,
                    ));
                }

                return Ok(());
            }

            "TEARDOWN" => {
                respond(&mut socket, &request, 200, "Session: m7s\r\n\r\n").await?;

                return Ok(());
            }

            _ => {
                respond(&mut socket, &request, 405, "").await?;
            }
        }
    }
}

struct Request {
    method: String,

    url: String,

    cseq: String,

    headers: String,
}

async fn read_request(socket: &mut TcpStream, buf: &mut Vec<u8>) -> Result<Request> {
    loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos + 4]).to_string();

            let mut lines = head.lines();

            let line = lines.next().unwrap_or("");

            let mut parts = line.split_whitespace();

            let method = parts.next().unwrap_or("").to_string();

            let url = parts.next().unwrap_or("").to_string();

            let cseq = lines
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;

                    if k.eq_ignore_ascii_case("CSeq") {
                        Some(v.trim().to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            buf.drain(..pos + 4);

            if method.is_empty() {
                bail!("empty RTSP request");
            }

            return Ok(Request {
                method,

                url,

                cseq,

                headers: head,
            });
        }

        let mut chunk = [0u8; 4096];

        let n = socket.read(&mut chunk).await.context("read RTSP")?;

        if n == 0 {
            bail!("RTSP closed");
        }

        buf.extend_from_slice(&chunk[..n]);

        if buf.len() > 64 * 1024 {
            bail!("RTSP request too large");
        }
    }
}

async fn respond(
    socket: &mut TcpStream,

    request: &Request,

    status: u16,

    extra: &str,
) -> Result<()> {
    let reason = match status {
        200 => "OK",

        404 => "Not Found",

        405 => "Method Not Allowed",

        _ => "Error",
    };

    let suffix = if extra.is_empty() { "\r\n" } else { "" };

    let response = format!(
        "RTSP/1.0 {status} {reason}\r\nCSeq: {}\r\n{extra}{suffix}",
        request.cseq
    );

    socket.write_all(response.as_bytes()).await?;

    socket.flush().await?;

    Ok(())
}

fn parse_path(url: &str) -> Option<(&str, &str)> {
    super::parse_path(url)
}


async fn read_rtcp_tcp(mut reader: ReadHalf<TcpStream>, stream: Arc<dyn TagStream>) {
    loop {
    let mut video_fu: Vec<u8> = Vec::new();
        let mut hdr = [0u8; 4];

        if reader.read_exact(&mut hdr).await.is_err() {
            break;
        }

        if hdr[0] != b'$' {
            continue;
        }

        let channel = hdr[1];

        let len = u16::from_be_bytes([hdr[2], hdr[3]]) as usize;

        if len > 1500 {
            break;
        }

        let mut payload = vec![0u8; len];

        if reader.read_exact(&mut payload).await.is_err() {
            break;
        }

        if channel == 0 {
            if let Some((ts, access)) = depacketize_video(&payload, &mut video_fu) {
                let (key, body) = annexb_to_avcc(&access);
                let mut data = vec![if key { 0x17 } else { 0x27 }, 0, 0, 0, 0];
                data.extend_from_slice(&body);
                stream.push_tag(9, ts / 90, data);
            }
        } else if channel == 1 || channel == 3 {
            if let Some(reports) = parse_rtcp_rr(&payload) {
                log::debug!("RTSP TCP RTCP RR: {:?}", reports);
            }
        } else if channel == 2 {
            for (ts, au) in depacketize_audio(&payload) {
                let mut data = vec![0xAF, 0x01];
                data.extend_from_slice(&au);
                stream.push_tag(8, ts / 90, data);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn send_udp_rtp(
    socket: &UdpSocket,

    client: SocketAddr,

    pt: u8,

    seq: &mut u16,

    ts: u32,

    ssrc: u32,

    payload: &[u8],

    marker: bool,
) -> Result<()> {
    let mut rtp = vec![0u8; 12];

    rtp[0] = 0x80;

    rtp[1] = if marker { 0x80 | pt } else { pt };

    rtp[2..4].copy_from_slice(&seq.to_be_bytes());

    rtp[4..8].copy_from_slice(&ts.to_be_bytes());

    rtp[8..12].copy_from_slice(&ssrc.to_be_bytes());

    *seq = seq.wrapping_add(1);

    rtp.extend_from_slice(payload);

    socket.send_to(&rtp, client).await?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn send_tag<W: AsyncWrite + Unpin>(
    socket: &mut W,

    tag: &FlvTag,

    seq: &mut u16,

    ssrc: &mut u32,

    aseq: &mut u16,

    assrc: u32,

    audio_ts: &mut u32,

    video_setup: bool,

    audio_setup: bool,

    sps: &[u8],

    pps: &[u8],

    vbase: u32,

    abase: u32,
) -> Result<()> {
    if tag.tag_type == 8 && tag.data.len() > 2 && tag.data[1] == 1 {
        if !audio_setup {
            return Ok(());
        }

        let frame = &tag.data[2..];

        let size = frame.len();

        let mut ts = tag.timestamp.wrapping_mul(90).wrapping_sub(abase);

        if ts <= *audio_ts {
            ts = audio_ts.wrapping_add(1);
        }

        *audio_ts = ts;

        let bits = size;

        let mut payload = vec![0, 16, (bits >> 5) as u8, ((bits & 0x1f) << 3) as u8];

        payload.extend_from_slice(frame);

        return send_rtp(socket, 2, 97, aseq, ts, assrc, &payload, true).await;
    }

    if !video_setup {
        return Ok(());
    }

    if tag.tag_type != 9 || tag.data.len() < 5 || (tag.data[0] & 0x0f) != 7 || tag.data[1] != 1 {
        return Ok(());
    }

    let mut pos = 5;

    let mut nalus = Vec::new();

    while pos + 4 <= tag.data.len() {
        let len = u32::from_be_bytes(tag.data[pos..pos + 4].try_into().unwrap()) as usize;

        pos += 4;

        if len == 0 || pos + len > tag.data.len() {
            break;
        }

        nalus.push(&tag.data[pos..pos + len]);

        pos += len;
    }

    let ts = tag.timestamp.wrapping_mul(90).wrapping_sub(vbase);

    if tag.data[1] == 1 {
        if !pps.is_empty() {
            nalus.insert(0, pps);
        }

        if !sps.is_empty() {
            nalus.insert(0, sps);
        }
    }

    let nal_count = nalus.len();

    for (idx, nalu) in nalus.iter().enumerate() {
        if nalu.is_empty() {
            continue;
        }

        let max = 1400;

        if nalu.len() <= max {
            send_rtp(socket, 0, 96, seq, ts, *ssrc, nalu, idx + 1 == nal_count).await?;
        } else {
            let mut off = 1;

            while off < nalu.len() {
                let end = (off + max - 2).min(nalu.len());

                let start = off == 1;

                let last = end == nalu.len();

                let mut payload = Vec::with_capacity(end - off + 2);

                payload.push((nalu[0] & 0xe0) | 28);

                payload.push((nalu[0] & 0x1f) | ((start as u8) << 7) | ((last as u8) << 6));

                payload.extend_from_slice(&nalu[off..end]);

                send_rtp(
                    socket,
                    0,
                    96,
                    seq,
                    ts,
                    *ssrc,
                    &payload,
                    last && idx + 1 == nal_count,
                )
                .await?;

                off = end;
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn send_rtp<W: AsyncWrite + Unpin>(
    socket: &mut W,

    channel: u8,

    pt: u8,

    seq: &mut u16,

    ts: u32,

    ssrc: u32,

    payload: &[u8],

    marker: bool,
) -> Result<()> {
    let mut rtp = vec![0u8; 12];

    rtp[0] = 0x80;

    rtp[1] = if marker { 0x80 | pt } else { pt };

    rtp[2..4].copy_from_slice(&seq.to_be_bytes());

    rtp[4..8].copy_from_slice(&ts.to_be_bytes());

    rtp[8..12].copy_from_slice(&ssrc.to_be_bytes());

    *seq = seq.wrapping_add(1);

    rtp.extend_from_slice(payload);

    let len = rtp.len() as u16;

    let mut out = Vec::with_capacity(4 + rtp.len());

    out.push(b'$');

    out.push(channel);

    out.extend_from_slice(&len.to_be_bytes());

    out.extend_from_slice(&rtp);

    socket.write_all(&out).await?;

    socket.flush().await?;

    Ok(())
}

fn track_id(url: &str) -> u8 {
    super::track_id(url)
}

pub fn avc_config(cache: &[SharedTag]) -> (Vec<u8>, Vec<u8>) {
    m7s_codec::avc_config(cache)
}

pub fn audio_config(cache: &[SharedTag]) -> Vec<u8> {
    m7s_codec::audio_config(cache)
}

fn trim_cache(cache: Vec<SharedTag>) -> Vec<SharedTag> {
    let key = cache
        .iter()
        .rposition(|t| {
            t.tag_type == 9 && t.data.len() >= 5 && (t.data[0] >> 4) == 1 && t.data[1] == 1
        })
        .unwrap_or(0);

    let start = cache[..=key]
        .iter()
        .rposition(|t| t.tag_type == 9 && t.data.len() >= 2 && t.data[1] == 0)
        .unwrap_or(key);

    cache.into_iter().skip(start).collect()
}

async fn play_loop(
    stream: Arc<dyn TagStream>,
    socket: TcpStream,
    mut reader: Box<dyn m7s_sdk::TagReader>,
    video_setup: bool,
    audio_setup: bool,
) {
    let mut initial = Vec::new();
    while let Some(t) = reader.try_next() {
        initial.push(Arc::new(FlvTag::new(t.tag_type, t.timestamp, t.data)));
    }
    let initial = trim_cache(initial);
    let (_sps, _pps) = avc_config(&initial);
    let vts = initial.iter().find(|t| t.tag_type == 9).map(|t| t.timestamp.wrapping_mul(90)).unwrap_or(0);
    let ats = initial.iter().find(|t| t.tag_type == 8).map(|t| t.timestamp.wrapping_mul(90)).unwrap_or(0);
    let mut seq = 0u16;
    let mut ssrc = 0x1234_5678u32;
    let mut aseq = 0u16;
    let assrc = 0x2233_4455u32;
    let mut audio_ts = 0u32;
    let (reader_tcp, writer) = split(socket);
    let mut writer = BufWriter::with_capacity(16384, writer);
    let reader_stream = stream.clone();
tokio::spawn(async move { read_rtcp_tcp(reader_tcp, reader_stream).await; });
    for tag in &initial {
        if send_tag(&mut writer, tag, &mut seq, &mut ssrc, &mut aseq, assrc, &mut audio_ts, video_setup, audio_setup, &[], &[], 0, 0).await.is_err() {
            stream.leave(); return;
        }
    }
    let _ = writer.flush().await;
    if video_setup { let _ = send_rtcp_tcp(&mut writer, 1, 0x1234_5678, vts).await; }
    if audio_setup { let _ = send_rtcp_tcp(&mut writer, 3, 0x2233_4455, ats).await; }
    loop {
        let mut got = false;
        while let Some(t) = reader.try_next() {
            got = true;
            let tag = FlvTag::new(t.tag_type, t.timestamp, t.data);
            if send_tag(&mut writer, &tag, &mut seq, &mut ssrc, &mut aseq, assrc, &mut audio_ts, video_setup, audio_setup, &[], &[], 0, 0).await.is_err() {
                stream.leave(); return;
            }
                let _ = writer.flush().await;
        }
        if !got { tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
    }
}

#[allow(clippy::too_many_arguments)]
async fn send_tag_udp(
    tag: &FlvTag,

    video: &Option<UdpSetup>,

    audio: &Option<UdpSetup>,

    vseq: &mut u16,

    aseq: &mut u16,

    audio_ts: &mut u32,

    video_setup: bool,

    audio_setup: bool,

    vbase: u32,

    abase: u32,
) -> Result<()> {
    let _ = vbase;

    let _ = abase;

    if tag.tag_type == 8 && tag.data.len() > 2 && tag.data[1] == 1 {
        if !audio_setup {
            return Ok(());
        }

        if let Some(a) = audio {
            let frame = &tag.data[2..];

            let size = frame.len();

            let mut ts = tag.timestamp.wrapping_mul(90).wrapping_sub(abase);

            if ts <= *audio_ts {
                ts = audio_ts.wrapping_add(1);
            }

            *audio_ts = ts;

            let bits = size;

            let mut payload = vec![0, 16, (bits >> 5) as u8, ((bits & 0x1f) << 3) as u8];

            payload.extend_from_slice(frame);

            send_udp_rtp(
                &a.rtp,
                a.client_rtp,
                97,
                aseq,
                ts,
                0x2233_4455,
                &payload,
                true,
            )
            .await?;
        }

        return Ok(());
    }

    if !video_setup {
        return Ok(());
    }

    let Some(v) = video else { return Ok(()) };

    if tag.tag_type != 9 || tag.data.len() < 5 || (tag.data[0] & 0x0f) != 7 || tag.data[1] != 1 {
        return Ok(());
    }

    let mut pos = 5;

    while pos + 4 <= tag.data.len() {
        let len = u32::from_be_bytes(tag.data[pos..pos + 4].try_into().unwrap()) as usize;

        pos += 4;

        if len == 0 || pos + len > tag.data.len() {
            break;
        }

        let nalu = &tag.data[pos..pos + len];

        pos += len;

        let max = 1400;

        if nalu.len() <= max {
            send_udp_rtp(
                &v.rtp,
                v.client_rtp,
                96,
                vseq,
                tag.timestamp.wrapping_mul(90),
                0x1122_3344,
                nalu,
                true,
            )
            .await?;
        } else {
            let mut off = 1;

            while off < nalu.len() {
                let end = (off + max - 2).min(nalu.len());

                let start = off == 1;

                let last = end == nalu.len();

                let mut payload = Vec::with_capacity(end - off + 2);

                payload.push((nalu[0] & 0xe0) | 28);

                payload.push((nalu[0] & 0x1f) | ((start as u8) << 7) | ((last as u8) << 6));

                payload.extend_from_slice(&nalu[off..end]);

                send_udp_rtp(
                    &v.rtp,
                    v.client_rtp,
                    96,
                    vseq,
                    tag.timestamp.wrapping_mul(90),
                    0x1122_3344,
                    &payload,
                    last,
                )
                .await?;

                off = end;
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn play_udp(
    stream: Arc<dyn TagStream>,
    _socket: TcpStream,
    video: Option<UdpSetup>,
    audio: Option<UdpSetup>,
    mut reader: Box<dyn m7s_sdk::TagReader>,
    video_setup: bool,
    audio_setup: bool,
) {
    let mut initial = Vec::new();
    while let Some(t) = reader.try_next() {
        initial.push(Arc::new(FlvTag::new(t.tag_type, t.timestamp, t.data)));
    }
    let initial = trim_cache(initial);
    let mut vseq = 0u16;
    let mut aseq = 0u16;
    let mut audio_ts = 0u32;
    for tag in &initial {
        if send_tag_udp(tag, &video, &audio, &mut vseq, &mut aseq, &mut audio_ts, video_setup, audio_setup, 0, 0).await.is_err() {
            stream.leave(); return;
        }
    }
    loop {
        let mut got = false;
        while let Some(t) = reader.try_next() {
            got = true;
            let tag = FlvTag::new(t.tag_type, t.timestamp, t.data);
            if send_tag_udp(&tag, &video, &audio, &mut vseq, &mut aseq, &mut audio_ts, video_setup, audio_setup, 0, 0).await.is_err() {
                stream.leave(); return;
            }
        }
        if !got { tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
    }
}

#[allow(dead_code)]
async fn send_rtcp_tcp<W: AsyncWrite + Unpin>(
    socket: &mut W,
    channel: u8,
    ssrc: u32,
    rtp_ts: u32,
) -> Result<()> {
    let packet = rtcp_sr(ssrc, rtp_ts);

    let mut out = Vec::with_capacity(4 + packet.len());

    out.push(b'$');

    out.push(channel);

    out.extend_from_slice(&(packet.len() as u16).to_be_bytes());

    out.extend_from_slice(&packet);

    socket.write_all(&out).await?;

    socket.flush().await?;

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RtcpReport {
    ssrc: u32,

    fraction_lost: u8,

    cumulative_lost: u32,

    highest_seq: u32,

    jitter: u32,
}

fn parse_rtcp_rr(packet: &[u8]) -> Option<Vec<RtcpReport>> {
    if packet.len() < 8 || packet[1] != 201 {
        return None;
    }

    let count = (packet[0] & 0x1f) as usize;

    let mut out = Vec::with_capacity(count);

    let mut pos = 8;

    for _ in 0..count {
        if pos + 24 > packet.len() {
            break;
        }

        let ssrc = u32::from_be_bytes(packet[pos..pos + 4].try_into().ok()?);

        let fraction_lost = packet[pos + 4];

        let cumulative_lost =
            u32::from_be_bytes([0, packet[pos + 5], packet[pos + 6], packet[pos + 7]]);

        let highest_seq = u32::from_be_bytes(packet[pos + 8..pos + 12].try_into().ok()?);

        let jitter = u32::from_be_bytes(packet[pos + 12..pos + 16].try_into().ok()?);

        out.push(RtcpReport {
            ssrc,

            fraction_lost,

            cumulative_lost,

            highest_seq,

            jitter,
        });

        pos += 24;
    }

    Some(out)
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn sdp_contains_h264() {
        let text = sdp(&[0x67, 1], &[0x68, 2], &[]);

        assert!(text.contains("H264/90000"));

        assert!(text.contains("sprop-parameter-sets="));
    }

    #[test]
    fn parses_rtcp_rr() {
        let mut packet = [0u8; 32];

        packet[0] = 0x81;

        packet[1] = 201;

        packet[2..4].copy_from_slice(&6u16.to_be_bytes());

        packet[4..8].copy_from_slice(&7u32.to_be_bytes());

        packet[8..12].copy_from_slice(&9u32.to_be_bytes());

        packet[12] = 5;

        let reports = parse_rtcp_rr(&packet).unwrap();

        assert_eq!(reports.len(), 1);

        assert_eq!(reports[0].ssrc, 9);

        assert_eq!(reports[0].fraction_lost, 5);
    }

    #[test]
    fn path() {
        assert_eq!(
            parse_path("rtsp://127.0.0.1:8554/live/test"),
            Some(("live", "test"))
        );
    }
}

fn digest_header(headers: &str) -> Option<String> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case("authorization")
            .then(|| value.trim().to_string())
    })
}

fn digest_param<'a>(value: &'a str, key: &str) -> Option<&'a str> {
    value.split(',').find_map(|part| {
        let (k, v) = part.trim().split_once('=')?;
        if k.trim() == key {
            Some(v.trim().trim_matches('"'))
        } else {
            None
        }
    })
}

fn md5_hex(input: &str) -> String {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(input.as_bytes());
    format!("{:x}", h.finalize())
}

static NONCES: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, i64>>> =
    std::sync::OnceLock::new();

fn nonce_store() -> &'static std::sync::Mutex<std::collections::HashMap<String, i64>> {
    NONCES.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn authorized(request: &Request, auth: &RtspAuth) -> bool {
    let Some(value) = digest_header(&request.headers) else {
        return false;
    };
    let value = value.strip_prefix("Digest ").unwrap_or(&value);
    let Some(user) = digest_param(value, "username") else {
        return false;
    };
    let Some(nonce) = digest_param(value, "nonce") else {
        return false;
    };
    let Some(response) = digest_param(value, "response") else {
        return false;
    };
    if user != auth.user {
        return false;
    }
    {
        let store = nonce_store().lock().unwrap_or_else(|e| e.into_inner());
        match store.get(nonce) {
            Some(expires) if *expires > now_ms() => {}
            _ => return false,
        }
    }
    let uri = digest_param(value, "uri").unwrap_or(&request.url);
    let ha1 = md5_hex(&format!("{}:m7s:{}", auth.user, auth.password));
    let ha2 = md5_hex(&format!("{}:{}", request.method, uri));
    let expected = if let Some(qop) = digest_param(value, "qop") {
        let nc = digest_param(value, "nc").unwrap_or("");
        let cnonce = digest_param(value, "cnonce").unwrap_or("");
        md5_hex(&format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}"))
    } else {
        md5_hex(&format!("{ha1}:{nonce}:{ha2}"))
    };
    response.eq_ignore_ascii_case(&expected)
}

async fn send_unauthorized(
    socket: &mut TcpStream,
    request: &Request,
) -> Result<()> {
    let nonce = format!("{:x}", now_ms());
    nonce_store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(nonce.clone(), now_ms() + 60_000);
    let response = format!(
        "RTSP/1.0 401 Unauthorized\r\nCSeq: {}\r\nWWW-Authenticate: Digest realm=\"m7s\", nonce=\"{nonce}\", qop=\"auth\"\r\n\r\n",
        request.cseq
    );
    socket.write_all(response.as_bytes()).await?;
    Ok(())
}

fn depacketize_audio(packet: &[u8]) -> Vec<(u32, Vec<u8>)> {
    if packet.len() < 16 {
        return Vec::new();
    }
    let ts = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);
    let offset = 12 + (packet[0] & 0x0f) as usize * 4;
    let payload = &packet[offset..];
    if payload.len() < 4 {
        return Vec::new();
    }
    let header_bits = u16::from_be_bytes([payload[0], payload[1]]) as usize;
    if header_bits == 0 || header_bits & 7 != 0 || header_bits / 8 + 2 > payload.len() {
        return Vec::new();
    }
    let header_len = header_bits / 8;
    let count = header_len / 2;
    let mut pos = 2 + header_len;
    let mut out = Vec::new();
    for i in 0..count {
        let h = 2 + i * 2;
        let size = (u16::from_be_bytes([payload[h], payload[h + 1]]) >> 3) as usize;
        if size == 0 || pos + size > payload.len() {
            break;
        }
        out.push((ts, payload[pos..pos + size].to_vec()));
        pos += size;
    }
    out
}

fn depacketize_video(packet: &[u8], fu: &mut Vec<u8>) -> Option<(u32, Vec<u8>)> {
    if packet.len() < 13 {
        return None;
    }
    let ts = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);
    let offset = 12 + (packet[0] & 0x0f) as usize * 4;
    let payload = &packet[offset..];
    if payload.is_empty() {
        return None;
    }
    let nal = payload[0] & 0x1f;
    if nal == 28 {
        if payload.len() < 3 {
            return None;
        }
        if payload[1] & 0x80 != 0 {
            fu.clear();
            fu.extend_from_slice(&[0, 0, 0, 1, (payload[0] & 0xe0) | (payload[1] & 0x1f)]);
        }
        fu.extend_from_slice(&payload[2..]);
        if payload[1] & 0x40 != 0 {
            return Some((ts, std::mem::take(fu)));
        }
        None
    } else if (1..=23).contains(&nal) {
        let mut out = vec![0, 0, 0, 1];
        out.extend_from_slice(payload);
        Some((ts, out))
    } else {
        None
    }
}

fn annexb_to_avcc(annexb: &[u8]) -> (bool, Vec<u8>) {
    let mut out = Vec::new();
    let mut i = 0;
    let mut key = false;
    while i + 3 < annexb.len() {
        let start = if annexb[i..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if annexb[i..].starts_with(&[0, 0, 1]) {
            3
        } else {
            i += 1;
            continue;
        };
        let begin = i + start;
        let mut end = annexb.len();
        let mut j = begin;
        while j + 3 < annexb.len() {
            if annexb[j..].starts_with(&[0, 0, 1]) || annexb[j..].starts_with(&[0, 0, 0, 1]) {
                end = j;
                break;
            }
            j += 1;
        }
        if begin < end {
            if annexb[begin] & 0x1f == 5 {
                key = true;
            }
            out.extend_from_slice(&((end - begin) as u32).to_be_bytes());
            out.extend_from_slice(&annexb[begin..end]);
        }
        i = end;
    }
    (key, out)
}
