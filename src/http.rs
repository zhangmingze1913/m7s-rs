//! HTTP surface: HTTP-FLV, HLS, Admin, stream API and cluster control plane.
//!
//! Layering:
//! 1. request read/parse and authentication;
//! 2. route dispatch returning serde_json values;
//! 3. streaming routes (HTTP-FLV/HLS/SSE) write the socket directly;
//! 4. response helpers at the bottom.
//!
//! Per-frame work belongs in `http_flv`/`hls`, not here.

use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::Duration;
use tokio_rustls::TlsAcceptor;

use crate::cluster::Cluster;
use crate::flv::FlvTag;
use crate::hls::HlsManager;
use crate::stream::StreamManager;

#[derive(Clone)]
pub struct AppState {
    pub streams: Arc<StreamManager>,
    pub cluster: Arc<Cluster>,
    pub hls: Arc<HlsManager>,
    pub node_id: String,
    pub api_token: String,
}

pub async fn run(listener: TcpListener, state: AppState) -> Result<()> {
    log::info!("HTTP listening on {}", listener.local_addr()?);
    loop {
        let (socket, peer) = listener.accept().await?;
        let _ = socket.set_nodelay(true);
        if let Ok(bytes) = std::env::var("M7S_HTTP_SNDBUF").and_then(|v| v.parse::<usize>().map_err(|_| std::env::VarError::NotPresent)) {
            let _ = socket2::SockRef::from(&socket).set_send_buffer_size(bytes.max(4096));
        }
        let Some(guard) = m7s_sdk::connections::try_acquire() else {
            drop(socket);
            continue;
        };
        let state = state.clone();
        tokio::spawn(async move {
            let _guard = guard;
            if let Err(e) = handle(socket, state).await {
                log::debug!("HTTP client {peer}: {e:#}");
            }
        });
    }
}

pub async fn run_tls(listener: TcpListener, state: AppState, acceptor: TlsAcceptor) -> Result<()> {
    log::info!("HTTPS listening on {}", listener.local_addr()?);
    loop {
        let (socket, peer) = listener.accept().await?;
        let _ = socket.set_nodelay(true);
        if let Ok(bytes) = std::env::var("M7S_HTTP_SNDBUF").and_then(|v| v.parse::<usize>().map_err(|_| std::env::VarError::NotPresent)) {
            let _ = socket2::SockRef::from(&socket).set_send_buffer_size(bytes.max(4096));
        }
        let Some(guard) = m7s_sdk::connections::try_acquire() else {
            drop(socket);
            continue;
        };
        let state = state.clone();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            let _guard = guard;
            match acceptor.accept(socket).await {
                Ok(stream) => {
                    if let Err(e) = handle(stream, state).await {
                        log::debug!("HTTPS client {peer}: {e:#}");
                    }
                }
                Err(e) => log::debug!("TLS handshake {peer}: {e}"),
            }
        });
    }
}

async fn handle<S>(mut socket: S, state: AppState) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut buf = vec![0u8; 16 * 1024];
    let mut used = 0;
    loop {
        let n = socket.read(&mut buf[used..]).await?;
        if n == 0 {
            return Ok(());
        }
        used += n;
        if buf[..used].windows(4).any(|w| w == b"\r\n\r\n") || used == buf.len() {
            break;
        }
    }
    let request = String::from_utf8_lossy(&buf[..used]);
    let mut lines = request.lines();
    let line = lines.next().unwrap_or("");
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    if method != "GET" {
        return respond_text(&mut socket, 405, "text/plain", "method not allowed").await;
    }
    let path = target.split('?').next().unwrap_or(target);
    let range = request.lines().find_map(|l| l.strip_prefix("Range: bytes=")).and_then(parse_range);
    let route = path
        .strip_prefix("/v6/api")
        .map(|rest| format!("/api{rest}"))
        .unwrap_or_else(|| path.to_string());
    // --- Authentication: admin token, JWT, or tenant token. ---
    let jwt_secret = std::env::var("M7S_JWT_SECRET").unwrap_or_default();
    let mut authorized = state.api_token.is_empty() && jwt_secret.is_empty();
    let mut caller = "anonymous".to_string();
    let mut role = if authorized { "admin".to_string() } else { "viewer".to_string() };
    if !authorized {
        for line in request.lines().skip(1) {
            if let Some(v) = line.strip_prefix("Authorization: Bearer ") {
                let v = v.trim();
                if v == state.api_token {
                    authorized = true;
                    caller = "api-token".to_string();
                    role = "admin".to_string();
                    break;
                }
                if let Some(t) = crate::tenant::by_token(v) {
                    authorized = true;
                    caller = t.name;
                    role = "tenant".to_string();
                    break;
                }
                if !jwt_secret.is_empty() {
                    if let Some(claims) = crate::auth::verify(&jwt_secret, v) {
                        authorized = true;
                        caller = claims.sub;
                        role = claims.role;
                        break;
                    }
                }
            } else if let Some(v) = line.strip_prefix("X-API-Token: ") {
                if v.trim() == state.api_token {
                    authorized = true;
                    caller = "api-token".to_string();
                    role = "admin".to_string();
                    break;
                }
                if let Some(t) = crate::tenant::by_token(v.trim()) {
                    authorized = true;
                    caller = t.name;
                    role = "tenant".to_string();
                    break;
                }
            }
        }
    }
    if (path.starts_with("/api")
        || path.starts_with("/v6/api")
        || path.starts_with("/cluster/api"))
        && !authorized
        && path != "/api/login"
        && path != "/v6/api/login"
    {
        return respond_json(
            &mut socket,
            401,
            &json!({"code":401,"message":"unauthorized"}),
        )
        .await;
    }
    let admin_only = path == "/api/plugins/toggle"
        || path == "/api/stream/delete"
        || path.starts_with("/api/users")
        || path.starts_with("/api/audit")
        || path.starts_with("/api/license/activate")
        || path.starts_with("/api/license/revoke")
        || path.starts_with("/api/admin")
        || path == "/api/config/reload"
        || path.starts_with("/api/tenants");
    if admin_only && role != "admin" {
        return respond_json(
            &mut socket,
            403,
            &json!({"code":403,"message":"forbidden"}),
        )
        .await;
    }
    // --- Streaming routes are handled before the JSON route match. ---
    if let Some((app, name)) = parse_flv_path(path) {
        if role == "tenant" {
            if app != caller {
                return respond_json(&mut socket, 403, &json!({"code":403,"message":"tenant stream isolation"})).await;
            }
            if let Some(t) = crate::tenant::get(&caller) {
                let viewers: usize = state.streams.list().iter().filter(|s| s["app"].as_str() == Some(caller.as_str())).map(|s| s["subscribers"].as_u64().unwrap_or(0) as usize).sum();
                if viewers >= t.max_viewers {
                    return respond_json(&mut socket, 429, &json!({"code":429,"message":"tenant viewer quota"})).await;
                }
            }
        }
        return crate::http_flv::serve(socket, state, &app, &name).await;
    }
    if let Some((app, name, file)) = parse_hls_path(path) {
        return serve_hls(socket, state, &app, &name, &file).await;
    }
    if let Some(rest) = path.strip_prefix("/fmp4/") {
        let mut parts = rest.split('/');
        let app = parts.next().unwrap_or("");
        let name = parts.next().unwrap_or("");
        let file = parts.next().unwrap_or("");
        let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
        let fm_key = format!("{app}/{name}");
        let Some(stream) = state.streams.get(app, name) else {
            return respond_text(&mut socket, 404, "text/plain", "stream not found").await;
        };
        let tags: Vec<m7s_codec::FlvTag> = stream
            .cache_snapshot()
            .iter()
            .map(|t| m7s_codec::FlvTag::new(t.tag_type, t.timestamp, t.data.clone()))
            .collect();
        let fm_encrypt = state.hls.encrypt();
        let fm_aes_key = state.hls.key_bytes();
        match file {
            "key.bin" => {
                if !fm_encrypt {
                    return respond_text(&mut socket, 404, "text/plain", "encryption disabled").await;
                }
                return respond_bytes(&mut socket, 200, "application/octet-stream", &fm_aes_key).await;
            }
            "av_init.mp4" => {
                let Some((init, _)) = crate::fmp4::pack_av_all(&tags) else {
                    return respond_text(&mut socket, 404, "text/plain", "no A/V").await;
                };
                return respond_bytes(&mut socket, 200, "video/mp4", &init).await;
            }
            "av.m4s" | "av-0.m4s" => {
                let Some((_, segs)) = crate::fmp4::pack_av_all(&tags) else {
                    return respond_text(&mut socket, 404, "text/plain", "no A/V").await;
                };
                let Some(seg) = segs.first() else {
                    return respond_text(&mut socket, 404, "text/plain", "no A/V").await;
                };
                let data = fmp4_segment(&seg.0, 0, fm_encrypt, &fm_aes_key);
                return respond_bytes_range(&mut socket, "video/mp4", &data, range).await;
            }
            "aud_init.mp4" => {
                let Some((init, _)) = crate::fmp4::pack_audio_all(&tags) else {
                    return respond_text(&mut socket, 404, "text/plain", "no audio").await;
                };
                return respond_bytes(&mut socket, 200, "video/mp4", &init).await;
            }
            "aud.m3u8" => {
                let Some((_, segs)) = crate::fmp4::pack_audio_all(&tags) else {
                    return respond_text(&mut socket, 404, "text/plain", "no audio").await;
                };
                let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:7\n");
                if fm_encrypt {
                    out.push_str("#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n");
                }
                out.push_str("#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-MAP:URI=\"aud_init.mp4\"\n");
                for (i, (_, d)) in segs.iter().enumerate() {
                    out.push_str(&format!("#EXTINF:{d:.3},\naud-{i}.m4s\n"));
                }
                out.push_str("#EXT-X-ENDLIST\n");
                return respond_text(&mut socket, 200, "application/vnd.apple.mpegurl", &out).await;
            }
            "aud.m4s" | "aud-0.m4s" => {
                let Some((_, segs)) = crate::fmp4::pack_audio_all(&tags) else {
                    return respond_text(&mut socket, 404, "text/plain", "no audio").await;
                };
                let Some(seg) = segs.first() else {
                    return respond_text(&mut socket, 404, "text/plain", "no audio").await;
                };
                let data = fmp4_segment(&seg.0, 0, fm_encrypt, &fm_aes_key);
                return respond_bytes_range(&mut socket, "video/mp4", &data, range).await;
            }
            f if f.starts_with("aud-") && f.ends_with(".m4s") => {
                let idx: usize = f[4..f.len() - 4].parse().unwrap_or(usize::MAX);
                let Some((_, segs)) = crate::fmp4::pack_audio_all(&tags) else {
                    return respond_text(&mut socket, 404, "text/plain", "no audio").await;
                };
                let Some(seg) = segs.get(idx) else {
                    return respond_text(&mut socket, 404, "text/plain", "segment not found").await;
                };
                let data = fmp4_segment(&seg.0, idx as u64, fm_encrypt, &fm_aes_key);
                return respond_bytes_range(&mut socket, "video/mp4", &data, range).await;
            }
            "av.m3u8" => {
                let Some((_, segs)) = crate::fmp4::pack_av_all(&tags) else {
                    return respond_text(&mut socket, 404, "text/plain", "no A/V").await;
                };
                let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:7\n");
                if fm_encrypt {
                    out.push_str("#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n");
                }
                out.push_str("#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-MAP:URI=\"av_init.mp4\"\n");
                for (i, (_, d)) in segs.iter().enumerate() {
                    out.push_str(&format!("#EXTINF:{d:.3},\nav-{i}.m4s\n"));
                }
                out.push_str("#EXT-X-ENDLIST\n");
                return respond_text(&mut socket, 200, "application/vnd.apple.mpegurl", &out).await;
            }
            "index.m3u8" => {
                let msn = query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("_HLS_msn="))
                    .and_then(|v| v.parse::<usize>().ok());
                let part = query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("_HLS_part="))
                    .and_then(|v| v.parse::<usize>().ok());
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                loop {
                    let tags: Vec<m7s_codec::FlvTag> = stream
                        .cache_snapshot()
                        .iter()
                        .map(|t| m7s_codec::FlvTag::new(t.tag_type, t.timestamp, t.data.clone()))
                        .collect();
                    if let Some(pk) = crate::fmp4::pack_cached(&fm_key, &tags) {
                        let ready = match msn {
                            None => true,
                            Some(m) => match pk.segments.get(m) {
                                None => false,
                                Some(seg) => part.map(|p| seg.parts.len() > p).unwrap_or(true),
                            },
                        };
                        if ready || std::time::Instant::now() >= deadline {
                            let playlist = crate::fmp4::playlist(&pk.segments);
                            let playlist = if fm_encrypt { fmp4_playlist_key(playlist) } else { playlist };
                            return respond_text(&mut socket, 200, "application/vnd.apple.mpegurl", &playlist).await;
                        }
                    } else if std::time::Instant::now() >= deadline {
                        return respond_text(&mut socket, 404, "text/plain", "no fmp4 GOP").await;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
            "init.mp4" => {
                let Some(pk) = crate::fmp4::pack_cached(&fm_key, &tags) else {
                    return respond_text(&mut socket, 404, "text/plain", "no fmp4 GOP").await;
                };
                return respond_bytes(&mut socket, 200, "video/mp4", &pk.init).await;
            }
            f if f.starts_with("av-") && f.ends_with(".m4s") => {
                let idx: usize = f[3..f.len() - 4].parse().unwrap_or(usize::MAX);
                let Some((_, segs)) = crate::fmp4::pack_av_all(&tags) else {
                    return respond_text(&mut socket, 404, "text/plain", "no A/V").await;
                };
                let Some(seg) = segs.get(idx) else {
                    return respond_text(&mut socket, 404, "text/plain", "segment not found").await;
                };
                let data = fmp4_segment(&seg.0, idx as u64, fm_encrypt, &fm_aes_key);
                return respond_bytes_range(&mut socket, "video/mp4", &data, range).await;
            }
            f if f.starts_with("part-") && f.ends_with(".m4s") => {
                let key = &f[5..f.len() - 4];
                let (i, j) = key.split_once('-').unwrap_or(("", ""));
                let (i, j): (usize, usize) = (i.parse().unwrap_or(usize::MAX), j.parse().unwrap_or(usize::MAX));
                let msn = query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("_HLS_msn="))
                    .and_then(|v| v.parse::<usize>().ok());
                let part_req = query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("_HLS_part="))
                    .and_then(|v| v.parse::<usize>().ok());
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                loop {
                    let tags: Vec<m7s_codec::FlvTag> = stream
                        .cache_snapshot()
                        .iter()
                        .map(|t| m7s_codec::FlvTag::new(t.tag_type, t.timestamp, t.data.clone()))
                        .collect();
                    if let Some(pk) = crate::fmp4::pack_cached(&fm_key, &tags) {
                        let ready = match (msn, part_req) {
                            (Some(m), Some(p)) => pk
                                .segments
                                .get(m)
                                .map(|s| s.parts.len() > p)
                                .unwrap_or(false),
                            _ => true,
                        };
                        if ready {
                            if let Some(part) = pk.segments.get(i).and_then(|s| s.parts.get(j)) {
                                let data = fmp4_segment(&part.data, i as u64, fm_encrypt, &fm_aes_key);
                                return respond_bytes_range(&mut socket, "video/mp4", &data, range).await;
                            }
                            return respond_text(&mut socket, 404, "text/plain", "part not found").await;
                        }
                    }
                    if std::time::Instant::now() >= deadline {
                        return respond_text(&mut socket, 404, "text/plain", "part not ready").await;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
            f if f.starts_with("seg-") && f.ends_with(".m4s") => {
                let idx: usize = f[4..f.len() - 4].parse().unwrap_or(usize::MAX);
                let Some(pk) = crate::fmp4::pack_cached(&fm_key, &tags) else {
                    return respond_text(&mut socket, 404, "text/plain", "no fmp4 GOP").await;
                };
                let Some(seg) = pk.segments.get(idx) else {
                    return respond_text(&mut socket, 404, "text/plain", "segment not found").await;
                };
                return respond_bytes_range(&mut socket, "video/mp4", &seg.data, range).await;
            }
            _ => return respond_text(&mut socket, 404, "text/plain", "not found").await,
        }
    }
    if path == "/admin-react" || path == "/admin-react/" {
        return respond_text(&mut socket, 200, "text/html; charset=utf-8", ADMIN_REACT_HTML).await;
    }
    if path == "/admin.js" {
        return respond_text(&mut socket, 200, "application/javascript", ADMIN_REACT_JS).await;
    }
    if path == "/admin" || path == "/admin/" {
        return respond_text(&mut socket, 200, "text/html; charset=utf-8", ADMIN_HTML).await;
    }
    if path == "/sdk.js" {
        return respond_text(&mut socket, 200, "application/javascript", SDK_JS).await;
    }
    if path == "/player" || path == "/player/" {
        return respond_text(&mut socket, 200, "text/html; charset=utf-8", PLAYER_HTML).await;
    }
    if let Some(file) = path.strip_prefix("/recordings/") {
        return match crate::recording::read(&crate::recording::record_dir(), file) {
            Some(data) => {
                let ct = if file.ends_with(".mp4") { "video/mp4" } else { "video/x-flv" };
                respond_bytes(&mut socket, 200, ct, &data).await
            }
            None => respond_text(&mut socket, 404, "text/plain", "recording not found").await,
        };
    }
    let body = match route.as_str() {
        "/debug/pprof" => {
            let streams = state.streams.count();
            json!({
                "code":0,
                "pid": std::process::id(),
                "streams": streams,
                "subscribers": state.streams.subscriber_count(),
                "dropped": crate::dispatcher::dropped_total(),
                "parallelism": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
                "note":"pprof-lite snapshot; use M7S_TRACING=1 for spans"
            })
        }
        "/health" | "/healthz" => json!({
            "status":"ok",
            "node_id": state.node_id,
            "streams": state.streams.count(),
            "subscribers": state.streams.subscriber_count(),
        }),
        "/metrics" => {
            let mut text = format!(
                "# HELP m7s_streams Current stream count\n# TYPE m7s_streams gauge\nm7s_streams {}\n# HELP m7s_subscribers Current subscriber count\n# TYPE m7s_subscribers gauge\nm7s_subscribers {}\n",
                state.streams.count(),
                state.streams.subscriber_count()
            );
            text.push_str(&format!(
                "# HELP m7s_active_connections Active protocol connections\n# TYPE m7s_active_connections gauge\nm7s_active_connections {}\n# HELP m7s_max_connections Configured connection limit\n# TYPE m7s_max_connections gauge\nm7s_max_connections {}\n",
                m7s_sdk::connections::active(),
                m7s_sdk::connections::limit()
            ));
            text.push_str("# HELP m7s_stream_bytes Per-stream bytes\n# TYPE m7s_stream_bytes counter\n");
            text.push_str("# HELP m7s_stream_subscribers Per-stream subscribers\n# TYPE m7s_stream_subscribers gauge\n");
            for s in state.streams.list() {
                let key = s["key"].as_str().unwrap_or("");
                let bytes = s["bytes"].as_u64().unwrap_or(0);
                let subs = s["subscribers"].as_u64().unwrap_or(0);
                text.push_str(&format!("m7s_stream_bytes{{key=\"{key}\"}} {bytes}\n"));
                text.push_str(&format!("m7s_stream_subscribers{{key=\"{key}\"}} {subs}\n"));
            }
            text.push_str("# HELP m7s_dispatch_dropped_total Frames dropped by dispatchers\n# TYPE m7s_dispatch_dropped_total counter\n");
            text.push_str(&format!("m7s_dispatch_dropped_total {}\n", crate::dispatcher::dropped_total()));
            return respond_text(&mut socket, 200, "text/plain; version=0.0.4", &text).await;
        }
        "/api/debug/stream" => {
            if std::env::var("M7S_DEBUG_STREAMS").as_deref() != Ok("1") {
                json!({"code":403,"message":"debug disabled"})
            } else {
                let key = target.split_once('?').map(|(_, q)| q).unwrap_or("");
                let key = key.split('&').find_map(|kv| kv.strip_prefix("key=")).unwrap_or("");
                let (app, name) = key.split_once('/').unwrap_or(("live", "test"));
                let _ = state.streams.get_or_create(app, name);
                json!({"code":0,"key":key})
            }
        }        "/api/connections" => json!({"active":m7s_sdk::connections::active(),"limit":m7s_sdk::connections::limit()}),
        "/api/media/info" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let key = q.split('&').find_map(|kv| kv.strip_prefix("stream=")).unwrap_or("");
            let (app, name) = key.split_once('/').unwrap_or(("live", key));
            let Some(stream) = state.streams.get(app, name) else {
                return respond_json(&mut socket, 404, &json!({"code":404,"message":"stream not found"})).await;
            };
            let tags = stream.cache_snapshot();
            let (sps, _pps) = m7s_codec::avc_config(&tags);
            let dims = m7s_codec::sps_dimensions(&sps);
            let asc = tags.iter().find(|t| t.tag_type == 8 && t.data.len() > 3 && t.data[1] == 0).and_then(|t| m7s_codec::parse_asc(&t.data[2..4]));
            json!({"code":0,"stream":key,"video":{"width":dims.map(|d| d.0),"height":dims.map(|d| d.1),"profile":m7s_codec::sps_profile_level(&sps).map(|p| p.0),"level":m7s_codec::sps_profile_level(&sps).map(|p| p.1),"fps":m7s_codec::sps_frame_rate(&sps).map(|f| (f * 100.0).round() / 100.0)},"audio":{"object_type":asc.map(|a| a.object_type),"sample_rate":asc.map(|a| a.sample_rate),"channels":asc.map(|a| a.channels)}})
        }
        "/api/events/stream" => {
            let mut rx = crate::events::subscribe();
            let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
            socket.write_all(head.as_bytes()).await?;
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let limit = q.split('&').find_map(|kv| kv.strip_prefix("count=")).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
            let mut sent = 0usize;
            loop {
                if limit > 0 && sent >= limit { break; }
                match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
                    Ok(Ok(event)) => {
                        socket.write_all(format!("data: {event}\n\n").as_bytes()).await?;
                        socket.flush().await?;
                        sent += 1;
                    }
                    Ok(Err(_)) => break,
                    Err(_) => {
                        socket.write_all(b": ping\n\n").await?;
                        socket.flush().await?;
                    }
                }
            }
            let _ = socket.shutdown().await;
            return Ok(());
        }
        "/api/tenants" => json!({"code":0,"tenants":crate::tenant::list()}),
        "/api/tenants/create" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let name = q.split('&').find_map(|kv| kv.strip_prefix("name=")).unwrap_or("");
            let token = q.split('&').find_map(|kv| kv.strip_prefix("token=")).unwrap_or("");
            let max_streams = q.split('&').find_map(|kv| kv.strip_prefix("max_streams=")).and_then(|v| v.parse().ok()).unwrap_or(10);
            let max_viewers = q.split('&').find_map(|kv| kv.strip_prefix("max_viewers=")).and_then(|v| v.parse().ok()).unwrap_or(1000);
            let ok = crate::tenant::create(name, token, max_streams, max_viewers);
            crate::audit::log(&caller, &role, "tenant.create", name, ok);
            json!({"code":0,"created":ok,"tenant":name})
        }
        "/api/tenants/delete" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let name = q.split('&').find_map(|kv| kv.strip_prefix("name=")).unwrap_or("");
            let removed = crate::tenant::remove(name);
            crate::audit::log(&caller, &role, "tenant.delete", name, removed);
            json!({"code":0,"removed":removed})
        }
        "/api/streams" => json!({"code":0,"data":{"streams":state.streams.list()}}),
        "/api/admin/overview" => {
            let plugins = crate::plugins::list();
            let enabled = plugins["plugins"]
                .as_array()
                .map(|a| a.iter().filter(|p| p["status"].as_str() == Some("enabled")).count())
                .unwrap_or(0);
            json!({
                "code": 0,
                "node_id": state.node_id,
                "streams": state.streams.count(),
                "subscribers": state.streams.subscriber_count(),
                "plugins": {"total": plugins["count"], "enabled": enabled},
                "rooms": m7s_plugin_live::room_infos_list().len(),
                "db": crate::meta_db::global().is_some(),
                "license": crate::license::status(state.streams.count() as u32),
                "actor": caller,
                "role": role,
            })
        }
        "/api/alerts/stream" => {
            let head = "HTTP/1.1 200 OK\\r\\nContent-Type: text/event-stream\\r\\nCache-Control: no-cache\\r\\nConnection: close\\r\\nAccess-Control-Allow-Origin: *\\r\\n\\r\\n";
            socket.write_all(head.as_bytes()).await?;
            for _ in 0..3 {
                let alerts = crate::alerts::evaluate(state.streams.count() as u64, crate::dispatcher::dropped_total());
                let payload = serde_json::to_string(&alerts).unwrap_or_else(|_| "[]".to_string());
                socket.write_all(format!("data: {payload}\\n\\n").as_bytes()).await?;
                socket.flush().await?;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let _ = socket.shutdown().await;
            return Ok(());
        }
        "/api/alerts" => json!({
            "code": 0,
            "alerts": crate::alerts::evaluate(
                state.streams.count() as u64,
                crate::dispatcher::dropped_total(),
            )
        }),
        "/api/audit/export" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let limit = q.split('&').find_map(|kv| kv.strip_prefix("limit=")).and_then(|v| v.parse().ok()).unwrap_or(1000).min(10000);
            return respond_text(&mut socket, 200, "text/csv", &crate::audit::export_csv(limit)).await;
        }
        "/api/audit" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let limit = q
                .split('&')
                .find_map(|kv| kv.strip_prefix("limit="))
                .and_then(|v| v.parse().ok())
                .unwrap_or(100usize)
                .min(1000);
            json!({"code":0,"entries":crate::audit::recent(limit)})
        }
        "/api/db/streams" => {
            let Some(db) = crate::meta_db::global() else {
                return respond_json(&mut socket, 200, &json!({"code":0,"enabled":false,"streams":[]})).await;
            };
            let list = state.streams.list();
            for s in &list {
                let _ = db.upsert_stream(
                    s["key"].as_str().unwrap_or(""),
                    s["app"].as_str().unwrap_or(""),
                    s["name"].as_str().unwrap_or(""),
                    s["bytes"].as_u64().unwrap_or(0),
                    s["subscribers"].as_u64().unwrap_or(0),
                );
            }
            json!({"code":0,"enabled":true,"streams":db.list_streams().unwrap_or_default()})
        }
        "/api/db/rooms" => {
            let Some(db) = crate::meta_db::global() else {
                return respond_json(&mut socket, 200, &json!({"code":0,"enabled":false,"rooms":[]})).await;
            };
            for r in m7s_plugin_live::room_infos_list() {
                let _ = db.upsert_room(&r.name, &r.title, r.created, r.closed);
            }
            json!({"code":0,"enabled":true,"rooms":db.list_rooms().unwrap_or_default()})
        }
        "/api/config/reload" => {
            match crate::config::reload() {
                Ok(cfg) => {
                    state.hls.reload(cfg.hls.clone());
                    crate::audit::log(&caller, &role, "config.reload", "hls", true);
                    json!({"code":0,"reloaded":true,"hls":{"enabled":cfg.hls.enabled,"segment_secs":cfg.hls.segment_secs,"max_segments":cfg.hls.max_segments,"encrypt":cfg.hls.encrypt}})
                }
                Err(e) => json!({"code":400,"reloaded":false,"error":e.to_string()}),
            }
        }
        "/api/stream/delete" => {
            let stream = target
                .split_once('?')
                .and_then(|(_, q)| q.split('&').find_map(|kv| kv.strip_prefix("stream=")))
                .unwrap_or("");
            let (app, name) = stream.split_once('/').unwrap_or(("", ""));
            let removed = state.streams.remove(app, name);
            crate::audit::log(&caller, &role, "stream.delete", stream, removed);
            json!({"code":0,"removed":removed})
        }
        "/api/whip/rid" => {
            let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let stream = query.split('&').find_map(|kv| kv.strip_prefix("stream=")).unwrap_or("");
            let rid = query.split('&').find_map(|kv| kv.strip_prefix("rid=")).unwrap_or("");
            m7s_plugin_webrtc::set_rid(stream, rid);
            json!({"code":0,"stream":stream,"rid":rid})
        }        "/api/webrtc/capabilities" => json!({
            "code": 0,
            "simulcast_rid": true,
            "auto_layer_switch": true,
            "remb": true,
            "twcc": true,
            "twcc_per_packet": true,
            "transport_cc": true,
            "svc": false,
            "svc_reason": "no SVC encoder in current H.264 stack"
        }),
        "/api/live/join" | "/api/live/leave" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let user = q.split('&').find_map(|kv| kv.strip_prefix("user=")).unwrap_or("");
            if path.ends_with("/join") {
                let ok = m7s_plugin_live::join_room(room, user);
                json!({"code":0,"joined":ok,"members":m7s_plugin_live::members(room)})
            } else {
                m7s_plugin_live::leave_room(room, user);
                json!({"code":0,"members":m7s_plugin_live::members(room)})
            }
        }
        "/api/live/gift/events" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let limit = q.split('&').find_map(|kv| kv.strip_prefix("limit=")).and_then(|v| v.parse().ok()).unwrap_or(100).min(1000);
            let events: Vec<serde_json::Value> = m7s_plugin_live::gift_events(room, limit).into_iter().map(|e| json!({"ts":e.ts,"user":e.user,"gift":e.gift,"count":e.count})).collect();
            json!({"code":0,"room":room,"events":events})
        }
        "/api/live/gift/ranking" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let by_value = q.split('&').find_map(|kv| kv.strip_prefix("by=")).map(|v| v == "value").unwrap_or(false);
            let period = q.split('&').find_map(|kv| kv.strip_prefix("period="));
            let rows = if let Some(period) = period {
                let ms = match period {
                    "hour" => 3_600_000,
                    "day" => 86_400_000,
                    "week" => 604_800_000,
                    _ => 86_400_000,
                };
                m7s_plugin_live::gift_ranking_period(room, ms)
            } else if by_value {
                m7s_plugin_live::gift_ranking_value(room)
            } else {
                m7s_plugin_live::gift_ranking(room)
            };
            let rank: Vec<serde_json::Value> = rows.into_iter().map(|(user, total)| json!({"user":user,"total":total})).collect();
            json!({"code":0,"room":room,"ranking":rank})
        }
        "/api/live/gift" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let user = q.split('&').find_map(|kv| kv.strip_prefix("user=")).unwrap_or("");
            let gift = q.split('&').find_map(|kv| kv.strip_prefix("gift=")).and_then(|v| v.parse().ok()).unwrap_or(0);
            let count = q.split('&').find_map(|kv| kv.strip_prefix("count=")).and_then(|v| v.parse().ok()).unwrap_or(1);
            let total = m7s_plugin_live::send_gift(room, user, gift, count);
            json!({"code":0,"total":total})
        }
        "/api/live/pk" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let side = q.split('&').find_map(|kv| kv.strip_prefix("side=")).and_then(|v| v.parse().ok()).unwrap_or(0);
            let points = q.split('&').find_map(|kv| kv.strip_prefix("points=")).and_then(|v| v.parse().ok()).unwrap_or(0);
            m7s_plugin_live::pk_add(room, side, points);
            let (a, b) = m7s_plugin_live::pk_score(room);
            json!({"code":0,"a":a,"b":b})
        }
        "/api/live/bot" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let user = q.split('&').find_map(|kv| kv.strip_prefix("user=")).unwrap_or("bot");
            let text = q.split('&').find_map(|kv| kv.strip_prefix("text=")).unwrap_or("hello");
            let script = q.split('&').find_map(|kv| kv.strip_prefix("script=")).unwrap_or("");
            let interval = q.split('&').find_map(|kv| kv.strip_prefix("interval=")).and_then(|v| v.parse().ok()).unwrap_or(1000);
            let messages: Vec<String> = text.split(',').filter(|s| !s.is_empty()).map(str::to_string).collect();
            let ok = if script.is_empty() {
                m7s_plugin_live::start_bot_room(room, user, messages, interval)
            } else {
                m7s_plugin_live::start_bot_room_script(room, user, script, interval)
            };
            json!({"code":0,"started":ok,"room":room})
        }
        "/api/live/bot/stop" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            json!({"code":0,"stopped":m7s_plugin_live::stop_bot_room(room)})
        }
        "/api/live/chat/send" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let user = q.split('&').find_map(|kv| kv.strip_prefix("user=")).unwrap_or("");
            let text = q.split('&').find_map(|kv| kv.strip_prefix("text=")).unwrap_or("");
            match m7s_plugin_live::send_chat(room, user, text) {
                Ok(msg) => json!({"code":0,"message":msg}),
                Err(e) => json!({"code":403,"error":e}),
            }
        }
        "/api/live/chat/list" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let limit = q.split('&').find_map(|kv| kv.strip_prefix("limit=")).and_then(|v| v.parse().ok()).unwrap_or(50usize).min(200);
            json!({"code":0,"messages":m7s_plugin_live::chat_history(room, limit)})
        }
        "/api/live/chat/clear" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            m7s_plugin_live::clear_chat(room);
            json!({"code":0,"room":room})
        }
        "/api/live/seat/take" | "/api/live/seat/leave" | "/api/live/seat/list" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let user = q.split('&').find_map(|kv| kv.strip_prefix("user=")).unwrap_or("");
            let index = q.split('&').find_map(|kv| kv.strip_prefix("index=")).and_then(|v| v.parse().ok()).unwrap_or(0);
            let ok = if path.ends_with("/take") {
                m7s_plugin_live::take_seat(room, user, index)
            } else if path.ends_with("/leave") {
                m7s_plugin_live::leave_seat(room, user)
            } else {
                true
            };
            json!({"code":0,"ok":ok,"seats":m7s_plugin_live::seats(room)})
        }
        "/api/live/kick" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let user = q.split('&').find_map(|kv| kv.strip_prefix("user=")).unwrap_or("");
            let ok = m7s_plugin_live::kick(room, user);
            json!({"code":0,"ok":ok,"members":m7s_plugin_live::members(room)})
        }
        "/api/live/mute" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let user = q.split('&').find_map(|kv| kv.strip_prefix("user=")).unwrap_or("");
            let on = q.split('&').find_map(|kv| kv.strip_prefix("on=")).map(|v| v == "1").unwrap_or(true);
            m7s_plugin_live::set_muted(room, user, on);
            json!({"code":0,"muted":m7s_plugin_live::is_muted(room, user)})
        }
        "/api/live/robots" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let count = q.split('&').find_map(|kv| kv.strip_prefix("count=")).and_then(|v| v.parse().ok()).unwrap_or(0);
            let total = m7s_plugin_live::add_robots(room, count);
            json!({"code":0,"robots":total})
        }
        "/api/live/rooms" => {
            let infos = m7s_plugin_live::room_infos_list();
            let mut rooms: Vec<serde_json::Value> = infos
                .iter()
                .map(|r| {
                    json!({
                        "name": r.name,
                        "title": r.title,
                        "created": r.created,
                        "closed": r.closed,
                        "members": m7s_plugin_live::members(&r.name),
                    })
                })
                .collect();
            for name in m7s_plugin_live::room_names() {
                if !infos.iter().any(|r| r.name == name) {
                    rooms.push(json!({
                        "name": name,
                        "title": "",
                        "created": 0,
                        "closed": false,
                        "members": m7s_plugin_live::members(&name),
                    }));
                }
            }
            json!({"code":0,"rooms":rooms})
        }
        "/api/live/room/create" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let title = q.split('&').find_map(|kv| kv.strip_prefix("title=")).unwrap_or("");
            let ok = m7s_plugin_live::create_room(room, title);
            json!({"code":0,"created":ok,"room":room})
        }
        "/api/live/room/close" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let ok = m7s_plugin_live::close_room(room);
            json!({"code":0,"closed":ok,"room":room})
        }
        "/api/meeting/rooms" => json!({"code":0,"rooms":m7s_plugin_meeting::room_names()}),        "/api/meeting/stream" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let stream = q.split('&').find_map(|kv| kv.strip_prefix("stream=")).unwrap_or("");
            if stream.is_empty() {
                json!({"code":0,"room":room,"stream":m7s_plugin_meeting::stream_for(room)})
            } else {
                m7s_plugin_meeting::set_stream(room, stream);
                json!({"code":0,"room":room,"stream":stream})
            }
        }
        "/api/recordings" => {
            let items: Vec<serde_json::Value> = crate::recording::list(&crate::recording::record_dir(), &["flv", "mp4"])
                .into_iter()
                .map(|(name, size, modified)| json!({"name":name,"size":size,"modified":modified,"url":format!("/recordings/{name}")}))
                .collect();
            json!({"code":0,"recordings":items})
        }
        "/api/meeting/recordings" => {
            let items: Vec<serde_json::Value> = crate::recording::list(&crate::recording::mp4_dir(), &["mp4"])
                .into_iter()
                .map(|(name, size, modified)| json!({"name":name,"size":size,"modified":modified}))
                .collect();
            json!({"code":0,"recordings":items})
        }
        "/api/meeting/playback" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let file = q.split('&').find_map(|kv| kv.strip_prefix("file=")).unwrap_or("");
            match crate::recording::read(&crate::recording::mp4_dir(), file) {
                Some(data) => return respond_bytes(&mut socket, 200, "video/mp4", &data).await,
                None => json!({"code":404,"message":"recording not found"}),
            }
        }
        "/api/meeting/record" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let secs = q.split('&').find_map(|kv| kv.strip_prefix("secs=")).and_then(|v| v.parse().ok()).unwrap_or(5u32);
            match m7s_plugin_meeting::stream_for(room) {
                Some(key) => {
                    let (app, name) = key.split_once('/').unwrap_or(("live", "test"));
                    match crate::mp4::record(app, name, secs).await {
                        Ok(path) => json!({"code":0,"room":room,"path":path}),
                        Err(e) => json!({"code":500,"message":e}),
                    }
                }
                None => json!({"code":404,"message":"room has no stream"}),
            }
        }        "/api/live/status" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let (a, b) = m7s_plugin_live::pk_score(room);
            json!({"code":0,"room":room,"members":m7s_plugin_live::members(room),"pk":[a,b],"seats":m7s_plugin_live::seats(room),"muted":m7s_plugin_live::muted_users(room),"robots":m7s_plugin_live::robots(room),"gifts":m7s_plugin_live::gift_total(room)})
        }
        "/api/meeting/join" | "/api/meeting/leave" | "/api/meeting/publish" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let user = q.split('&').find_map(|kv| kv.strip_prefix("user=")).unwrap_or("");
            let on = q.split('&').find_map(|kv| kv.strip_prefix("on=")).map(|v| v == "1").unwrap_or(true);
            if path.ends_with("/join") {
                m7s_plugin_meeting::join_room(room, user);
            } else if path.ends_with("/leave") {
                m7s_plugin_meeting::leave_room(room, user);
            } else {
                m7s_plugin_meeting::publish(room, user, on);
            }
            json!({"code":0,"members":m7s_plugin_meeting::members(room),"publishers":m7s_plugin_meeting::publishers(room)})
        }
        "/api/meeting/mute" | "/api/meeting/hand" | "/api/meeting/kick" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let user = q.split('&').find_map(|kv| kv.strip_prefix("user=")).unwrap_or("");
            let on = q.split('&').find_map(|kv| kv.strip_prefix("on=")).map(|v| v == "1").unwrap_or(true);
            if path.ends_with("/mute") { m7s_plugin_meeting::set_muted(room, user, on); }
            else if path.ends_with("/hand") { m7s_plugin_meeting::raise_hand(room, user, on); }
            else { m7s_plugin_meeting::kick(room, user); }
            json!({"code":0,"room":room,"user":user,"muted":m7s_plugin_meeting::is_muted(room,user),"hands":m7s_plugin_meeting::hands(room)})
        }
        "/api/meeting/status" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            json!({"code":0,"room":room,"members":m7s_plugin_meeting::members(room),"publishers":m7s_plugin_meeting::publishers(room),"muted":m7s_plugin_meeting::muted_count(room),"hands":m7s_plugin_meeting::hands(room)})
        }        "/api/wasm/load" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let name = q.split('&').find_map(|kv| kv.strip_prefix("name=")).unwrap_or("");
            let path = q.split('&').find_map(|kv| kv.strip_prefix("path=")).unwrap_or("");
            match crate::wasm_plugin::load_file(name, path) {
                Ok(()) => json!({"code":0,"name":name}),
                Err(e) => json!({"code":500,"message":e}),
            }
        }
        "/api/wasm/av" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let name = q.split('&').find_map(|kv| kv.strip_prefix("name=")).unwrap_or("");
            let tag = q.split('&').find_map(|kv| kv.strip_prefix("tag=")).and_then(|v| v.parse().ok()).unwrap_or(9u8);
            let ts = q.split('&').find_map(|kv| kv.strip_prefix("ts=")).and_then(|v| v.parse().ok()).unwrap_or(0u32);
            let key = q.split('&').find_map(|kv| kv.strip_prefix("key=")).map(|v| v == "1").unwrap_or(false);
            let data = q.split('&').find_map(|kv| kv.strip_prefix("data=")).unwrap_or("");
            let frame = crate::wasm_plugin::AvFrame { tag_type: tag, keyframe: key, timestamp: ts, data: data.as_bytes().to_vec() };
            match crate::wasm_plugin::process_av_registered(name, &frame) {
                Ok(Some(out)) => json!({"code":0,"tag":out.tag_type,"key":out.keyframe,"ts":out.timestamp,"hex":crate::wasm_plugin::hex(&out.data)}),
                Ok(None) => json!({"code":0,"skipped":true}),
                Err(e) => json!({"code":500,"message":e}),
            }
        }        "/api/wasm/attach" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let name = q.split('&').find_map(|kv| kv.strip_prefix("name=")).unwrap_or("");
            let src = q.split('&').find_map(|kv| kv.strip_prefix("source=")).unwrap_or("");
            let dst = q.split('&').find_map(|kv| kv.strip_prefix("sink=")).unwrap_or("");
            let (sa, sn) = src.split_once('/').unwrap_or(("live", "src"));
            let (da, dn) = dst.split_once('/').unwrap_or(("live", "dst"));
            let source = state.streams.get_or_create(sa, sn);
            let sink = state.streams.get_or_create(da, dn);
            match crate::wasm_plugin::attach_registered(name, source, sink) {
                Ok(_) => json!({"code":0,"name":name,"source":src,"sink":dst}),
                Err(e) => json!({"code":500,"message":e}),
            }
        }        "/api/wasm/chains" => json!({"code":0,"chains":crate::wasm_plugin::chain_status()}),
        "/api/wasm/list" => {
            let plugins: Vec<serde_json::Value> = crate::wasm_plugin::status()
                .into_iter()
                .map(|(n, e)| json!({"name": n, "enabled": e}))
                .collect();
            json!({"code":0,"plugins":plugins})
        }
        "/api/wasm/enable" => {
            let q = target.split_once('?') .map(|(_, q)| q).unwrap_or("");
            let name = q.split('&').find_map(|kv| kv.strip_prefix("name=")).unwrap_or("");
            let enabled = q.split('&').find_map(|kv| kv.strip_prefix("enabled=")).map(|v| v == "1").unwrap_or(true);
            let ok = crate::wasm_plugin::set_enabled(name, enabled);
            json!({"code":0,"updated":ok,"enabled":enabled})
        }
        "/api/wasm/frame" => {
            let q = target.split_once('?') .map(|(_, q)| q).unwrap_or("");
            let name = q.split('&').find_map(|kv| kv.strip_prefix("name=")).unwrap_or("");
            let tag = q.split('&').find_map(|kv| kv.strip_prefix("tag=")).and_then(|v| v.parse().ok()).unwrap_or(9u8);
            let ts = q.split('&').find_map(|kv| kv.strip_prefix("ts=")).and_then(|v| v.parse().ok()).unwrap_or(0u32);
            let data = q.split('&').find_map(|kv| kv.strip_prefix("data=")).unwrap_or("");
            match crate::wasm_plugin::process_frame_registered(name, tag, ts, data.as_bytes()) {
                Ok(out) => json!({"code":0,"hex":crate::wasm_plugin::hex(&out)}),
                Err(e) => json!({"code":500,"message":e}),
            }
        }
        "/api/wasm/reload" => {
            let q = target.split_once('?') .map(|(_, q)| q).unwrap_or("");
            let name = q.split('&').find_map(|kv| kv.strip_prefix("name=")).unwrap_or("");
            match crate::wasm_plugin::reload(name) {
                Ok(()) => json!({"code":0,"name":name}),
                Err(e) => json!({"code":500,"message":e}),
            }
        }
        "/api/wasm/unload" => {
            let q = target.split_once('?') .map(|(_, q)| q).unwrap_or("");
            let name = q.split('&').find_map(|kv| kv.strip_prefix("name=")).unwrap_or("");
            let removed = crate::wasm_plugin::unload(name);
            json!({"code":0,"removed":removed})
        }
        "/api/wasm/process" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let name = q.split('&').find_map(|kv| kv.strip_prefix("name=")).unwrap_or("");
            let data = q.split('&').find_map(|kv| kv.strip_prefix("data=")).unwrap_or("");
            match crate::wasm_plugin::process_registered(name, data.as_bytes()) {
                Ok(out) => json!({"code":0,"hex":crate::wasm_plugin::hex(&out)}),
                Err(e) => json!({"code":500,"message":e}),
            }
        }        "/api/plugins" => crate::plugins::list(),
        "/api/plugins/toggle" => {
            let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let name = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("name="))
                .unwrap_or("");
            let enabled = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("enabled="))
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            let result = crate::plugins::set_enabled(name, enabled);
            let ok = result.is_ok();
            crate::audit::log(&caller, &role, "plugin.toggle", name, ok);
            match result {
                Ok(disabled) => json!({"code":0,"name":name,"enabled":enabled,"disabled":disabled}),
                Err(e) => json!({"code":500,"message":e}),
            }
        }
        "/api/license" => crate::license::status(state.streams.count() as u32),
        "/api/license/activate" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let token = q.split('&').find_map(|kv| kv.strip_prefix("token=")).unwrap_or("");
            match crate::license::activate(token) {
                Ok(c) => json!({"code":0,"edition":c.edition,"expires":c.expires,"seats":c.seats}),
                Err(e) => json!({"code":400,"message":e}),
            }
        }
        "/api/license/activate-remote" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let server = q.split('&').find_map(|kv| kv.strip_prefix("server=")).unwrap_or("");
            let token = q.split('&').find_map(|kv| kv.strip_prefix("token=")).unwrap_or("");
            match crate::license::activate_remote(server, token).await {
                Ok(c) => json!({"code":0,"edition":c.edition,"expires":c.expires}),
                Err(e) => json!({"code":400,"message":e}),
            }
        }
        "/api/license/revoke" => {
            crate::license::revoke();
            json!({"code":0})
        }
        "/api/login" => {
            let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let username = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("username="))
                .unwrap_or("admin");
            let password = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("password="))
                .unwrap_or("");
            let secret = std::env::var("M7S_JWT_SECRET").unwrap_or_default();
            let env_ok = crate::users::count() == 0
                && username == "admin"
                && !password.is_empty()
                && std::env::var("M7S_ADMIN_PASSWORD").ok().as_deref() == Some(password);
            let mysql_ok = if std::env::var("M7S_MYSQL_URL").is_ok() {
                crate::mysql_users::check(username, password).await.unwrap_or(false)
            } else {
                false
            };
            let role = if mysql_ok {
                Some("admin".to_string())
            } else if crate::users::check(username, password) {
                Some(crate::users::role(username).unwrap_or_else(|| "viewer".to_string()))
            } else if env_ok {
                Some("admin".to_string())
            } else {
                None
            };
            match role {
                Some(role) => json!({"code":0,"token":crate::auth::issue_with_role(&secret, username, &role, 3600),"role":role}),
                None => json!({"code":401,"message":"invalid username or password"}),
            }
        }
        "/api/users/add" => {
            let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let username = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("username="))
                .unwrap_or("");
            let password = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("password="))
                .unwrap_or("");
            let new_role = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("role="))
                .unwrap_or("admin");
            if username.is_empty() || password.is_empty() {
                json!({"code":400,"message":"username/password required"})
            } else {
                let result = if std::env::var("M7S_MYSQL_URL").is_ok() {
                    crate::mysql_users::add(username, password).await
                } else {
                    crate::users::add_with_role(username, password, new_role).map_err(|e| e.to_string())
                };
                crate::audit::log(&caller, &role, "user.add", username, result.is_ok());
                match result {
                    Ok(()) => json!({"code":0,"username":username,"role":new_role}),
                    Err(e) => json!({"code":500,"message":e.to_string()}),
                }
            }
        }
        "/api/record/mp4" => {
            let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let stream = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("stream="))
                .unwrap_or("live/test");
            let secs = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("secs="))
                .and_then(|v| v.parse().ok())
                .unwrap_or(5);
            let (app, name) = stream.split_once('/').unwrap_or(("live", "test"));
            match crate::mp4::record(app, name, secs).await {
                Ok(path) => json!({"code":0,"path":path}),
                Err(e) => json!({"code":500,"message":e}),
            }
        }
        "/api/live/mix" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let room = q.split('&').find_map(|kv| kv.strip_prefix("room=")).unwrap_or("");
            let other = q.split('&').find_map(|kv| kv.strip_prefix("other=")).unwrap_or("");
            let secs = q.split('&').find_map(|kv| kv.strip_prefix("secs=")).and_then(|v| v.parse().ok()).unwrap_or(2);
            match crate::room_mix::mix_rooms(room, other, secs).await {
                Ok(path) => json!({"code":0,"path":path.to_string_lossy()}),
                Err(e) => json!({"code":400,"message":e}),
            }
        }
        "/api/sei" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let stream_key = q.split('&').find_map(|kv| kv.strip_prefix("stream=")).unwrap_or("");
            let text = q.split('&').find_map(|kv| kv.strip_prefix("text=")).unwrap_or("");
            let (app, name) = stream_key.split_once('/').unwrap_or(("live", stream_key));
            let Some(stream) = state.streams.get(app, name) else {
                return respond_json(&mut socket, 404, &json!({"code":404,"message":"stream not found"})).await;
            };
            let mut body = vec![7u8; 16];
            body.extend_from_slice(text.as_bytes());
            let mut sei = vec![0, 0, 1, 0x06, 0x05];
            let mut len = body.len();
            while len >= 255 {
                sei.push(255);
                len -= 255;
            }
            sei.push(len as u8);
            sei.extend_from_slice(&body);
            sei.push(0x80);
            let (_, avcc) = m7s_codec::annexb_to_avcc(&sei);
            let mut data = vec![0x27, 0, 0, 0, 0];
            data.extend_from_slice(&avcc);
            let ts = stream.cache_snapshot().iter().rev().find(|t| t.tag_type == 9).map(|t| t.timestamp).unwrap_or(0);
            m7s_sdk::TagSink::push_tag(stream.as_ref(), 9, ts, data);
            json!({"code":0,"stream":stream_key,"bytes":sei.len()})
        }
        "/api/mix" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let raw = q.split('&').find_map(|kv| kv.strip_prefix("input=")).unwrap_or("");
            let secs = q.split('&').find_map(|kv| kv.strip_prefix("secs=")).and_then(|v| v.parse().ok()).unwrap_or(1);
            let inputs: Vec<String> = raw.split(',').filter(|s| !s.is_empty()).map(str::to_string).collect();
            if inputs.len() < 2 {
                return respond_json(&mut socket, 400, &json!({"code":400,"message":"input=a,b required"})).await;
            }
            let result = tokio::task::spawn_blocking(move || m7s_plugin_mix::mix_bytes(&inputs, secs)).await;
            match result {
                Ok(Ok(data)) => {
                    let dir = std::path::Path::new("data/mix");
                    let name = format!("mix-{}.mp4", crate::time::now_ms());
                    let path = dir.join(&name);
                    if let Err(e) = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&path, &data)) {
                        return respond_json(&mut socket, 500, &json!({"code":500,"message":e.to_string()})).await;
                    }
                    json!({"code":0,"path":path.to_string_lossy(),"size":data.len()})
                }
                Ok(Err(e)) => json!({"code":500,"message":e}),
                Err(e) => json!({"code":500,"message":e.to_string()}),
            }
        }
        "/api/transcode" => {
            let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let stream = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("stream="))
                .unwrap_or("live/test");
            let secs = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("secs="))
                .and_then(|v| v.parse().ok())
                .unwrap_or(3);
            let width = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("width="))
                .and_then(|v| v.parse().ok())
                .unwrap_or(640);
            let (app, name) = stream.split_once('/').unwrap_or(("live", "test"));
            match crate::transcode::run(app, name, secs, width).await {
                Ok(path) => json!({"code":0,"path":path}),
                Err(e) => json!({"code":500,"message":e}),
            }
        }
        "/api/onvif/event" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let topic = q.split('&').find_map(|kv| kv.strip_prefix("topic=")).unwrap_or("tns1:VideoSource/MotionAlarm");
            let value = q.split('&').find_map(|kv| kv.strip_prefix("value=")).unwrap_or("true");
            m7s_plugin_onvif::push_event(topic, value);
            json!({"code":0,"topic":topic,"value":value})
        }
        "/api/onvif/pull" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let key = q.split('&').find_map(|kv| kv.strip_prefix("stream=")).unwrap_or("");
            let endpoint = q.split('&').find_map(|kv| kv.strip_prefix("endpoint=")).unwrap_or("");
            let profile = q.split('&').find_map(|kv| kv.strip_prefix("profile=")).unwrap_or("profile1");
            let user = q.split('&').find_map(|kv| kv.strip_prefix("user=")).unwrap_or("");
            let password = q.split('&').find_map(|kv| kv.strip_prefix("password=")).unwrap_or("");
            let (app, name) = key.split_once('/').unwrap_or(("", ""));
            if app.is_empty() || name.is_empty() || endpoint.is_empty() {
                return respond_json(&mut socket, 400, &json!({"code":400,"message":"stream=app/name&endpoint=required"})).await;
            }
            let response_endpoint = endpoint.to_string();
            let streams = state.streams.clone();
            let (endpoint, profile, user, password) = (endpoint.to_string(), profile.to_string(), user.to_string(), password.to_string());
            let (app, name) = (app.to_string(), name.to_string());
            tokio::spawn(async move {
                if let Err(e) = crate::onvif_pull::pull(&endpoint, &profile, &user, &password, streams, &app, &name).await {
                    log::warn!("ONVIF pull {endpoint} -> {app}/{name}: {e:#}");
                }
            });
            json!({"code":0,"stream":key,"endpoint":response_endpoint})
        }
        "/api/rtsp/pull" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let key = q.split('&').find_map(|kv| kv.strip_prefix("stream=")).unwrap_or("");
            let url = q.split('&').find_map(|kv| kv.strip_prefix("url=")).unwrap_or("");
            let (app, name) = key.split_once('/').unwrap_or(("", ""));
            if app.is_empty() || name.is_empty() || url.is_empty() {
                return respond_json(&mut socket, 400, &json!({"code":400,"message":"stream=app/name&url=required"})).await;
            }
            let streams = state.streams.clone();
            let response_url = url.to_string();
            let url = url.to_string();
            let (app, name) = (app.to_string(), name.to_string());
            tokio::spawn(async move {
                if let Err(e) = crate::rtsp_pull::pull(&url, streams, &app, &name).await {
                    log::warn!("RTSP pull {url}: {e:#}");
                }
            });
            json!({"code":0,"stream":key,"url":response_url})
        }
        "/api/webtransport/push" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let key = q.split('&').find_map(|kv| kv.strip_prefix("stream=")).unwrap_or("");
            let url = q.split('&').find_map(|kv| kv.strip_prefix("url=")).unwrap_or("");
            let (app, name) = key.split_once('/').unwrap_or(("", ""));
            let Some(stream) = state.streams.get(app, name) else {
                return respond_json(&mut socket, 404, &json!({"code": 404, "message": "stream not found"})).await;
            };
            let target = url.to_string();
            let egress_target = target.clone();
            let egress_key = key.to_string();
            tokio::spawn(async move {
                if let Err(e) = m7s_plugin_webtransport::push(stream, &egress_target).await {
                    log::warn!("WebTransport egress {egress_key} -> {egress_target}: {e:#}");
                }
            });
            json!({"code": 0, "stream": key, "target": target})
        }
        "/api/srt/push" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let key = q.split('&').find_map(|kv| kv.strip_prefix("stream=")).unwrap_or("");
            let url = q.split('&').find_map(|kv| kv.strip_prefix("url=")).unwrap_or("");
            let id = q.split('&').find_map(|kv| kv.strip_prefix("id=")).unwrap_or("");
            let (app, name) = key.split_once('/').unwrap_or(("", ""));
            let Some(stream) = state.streams.get(app, name) else {
                return respond_json(&mut socket, 404, &json!({"code": 404, "message": "stream not found"})).await;
            };
            let target = url.strip_prefix("srt://").unwrap_or(url).to_string();
            let stream_id = if id.is_empty() { key.to_string() } else { id.to_string() };
            let egress_target = target.clone();
            let egress_id = stream_id.clone();
            tokio::spawn(async move {
                if let Err(e) = m7s_plugin_srt::push(stream, &egress_target, &egress_id).await {
                    log::warn!("SRT egress {egress_id} -> {egress_target}: {e:#}");
                }
            });
            json!({"code": 0, "stream": key, "target": target, "stream_id": stream_id})
        }
        "/api/snapshot" => {
            let stream = target
                .split_once('?')
                .and_then(|(_, q)| q.split('&').find_map(|kv| kv.strip_prefix("stream=")))
                .unwrap_or("live/test");
            let (app, name) = stream.split_once('/').unwrap_or(("live", "test"));
            match crate::snap::capture(app, name).await {
                Ok(path) => json!({"code":0,"path":path}),
                Err(e) => json!({"code":500,"message":e}),
            }
        }
        "/api/dynamic/info" => crate::dynamic::info(),
        "/api/dynamic" => {
            let msg = target
                .split_once('?')
                .and_then(|(_, q)| q.split('&').find_map(|kv| kv.strip_prefix("msg=")))
                .unwrap_or("");
            match crate::dynamic::echo(msg) {
                Some(text) => json!({"loaded": true, "echo": text}),
                None => json!({"loaded": false}),
            }
        }
        "/api/hls" => json!({"code":0,"data":{"hls":true}}),
        "/cluster/api/status/local" => state.cluster.status_json(),
        "/cluster/api/servers" => state.cluster.servers_json(),
        "/cluster/api/sessions" => state.cluster.sessions_json(),
        "/cluster/api/relay/sessions" => state.cluster.relays_json(),
        "/cluster/api/allocate/play" => {
            let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
            let app = q.split('&').find_map(|kv| kv.strip_prefix("app=")).unwrap_or("live");
            let name = q.split('&').find_map(|kv| kv.strip_prefix("stream=")).unwrap_or("");
            let (server_id, redirect, address) = state.cluster.allocate_play(app, name);
            json!({
                "code":0,
                "data":{"server_id":server_id,"redirect":redirect,"address":address,"app":app,"stream":name}
            })
        },
        _ => {
            return respond_json(&mut socket, 404, &json!({"code":404,"message":"not found"}))
                .await;
        }
    };
    respond_json(&mut socket, 200, &body).await
}

fn parse_flv_path(path: &str) -> Option<(String, String)> {
    let rest = path.strip_prefix("/flv/")?;
    let rest = rest.strip_suffix(".flv").unwrap_or(rest);
    let (app, name) = rest.split_once('/')?;
    Some((app.to_string(), name.to_string()))
}

fn parse_hls_path(path: &str) -> Option<(String, String, String)> {
    let rest = path.strip_prefix("/hls/")?;
    let mut parts = rest.split('/');
    let app = parts.next()?;
    let name = parts.next()?;
    let file = parts.next()?;
    Some((app.to_string(), name.to_string(), file.to_string()))
}

async fn serve_hls<S>(
    mut socket: S,
    state: AppState,
    app: &str,
    name: &str,
    file: &str,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    if state.streams.get(app, name).is_none() {
        let _ = state.cluster.ensure_relay(app, name).await;
    }
    if !crate::plugins::enabled("hls") {
        return respond_text(&mut socket, 404, "text/plain", "HLS plugin disabled").await;
    }
    let Some(hls) = state.hls.ensure(app, name) else {
        return respond_text(&mut socket, 404, "text/plain", "HLS disabled").await;
    };
    if file == "master.m3u8" {
        let master = hls_master(&state, app, name);
        return match master {
            Some(text) => respond_text(&mut socket, 200, "application/vnd.apple.mpegurl", &text).await,
            None => respond_text(&mut socket, 404, "text/plain", "no HLS variants").await,
        };
    }
    if file == "key.bin" {
        return respond_bytes(&mut socket, 200, "application/octet-stream", &hls.key()).await;
    }
    if file == "index.m3u8" {
        let playlist = hls.playlist(4);
        return respond_text(&mut socket, 200, "application/vnd.apple.mpegurl", &playlist).await;
    }
    let seq = file
        .strip_prefix("seg-")
        .and_then(|s| s.strip_suffix(".ts"))
        .and_then(|s| s.parse::<u64>().ok());
    match seq.and_then(|seq| hls.segment(seq)) {
        Some(data) => {
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
                        data.len()
                    )
                    .as_bytes(),
                )
                .await?;
            socket.write_all(data.as_slice()).await?;
            socket.flush().await?;
            Ok(())
        }
        None => respond_text(&mut socket, 404, "text/plain", "segment not found").await,
    }
}

pub(crate) // --- HTTP response helpers. ---
async fn respond_json<S>(socket: &mut S, status: u16, body: &Value) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let bytes = serde_json::to_vec(body)?;
    respond_text(
        socket,
        status,
        "application/json",
        &String::from_utf8_lossy(&bytes),
    )
    .await
}

async fn respond_text<S>(socket: &mut S, status: u16, content_type: &str, body: &str) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        body.len()
    );
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(body.as_bytes()).await?;
    socket.flush().await?;
    let _ = socket.shutdown().await;
    Ok(())
}

async fn respond_bytes<S>(socket: &mut S, status: u16, content_type: &str, body: &[u8]) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let reason = if status == 200 { "OK" } else { "Error" };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        body.len()
    );
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(body).await?;
    socket.flush().await?;
    let _ = socket.shutdown().await;
    Ok(())
}
/// Feed a synthetic FLV stream into the local manager (tests / ingest adapters).
pub fn push_tag(streams: &StreamManager, app: &str, name: &str, tag: FlvTag) {
    streams.get_or_create(app, name).push(Arc::new(tag));
}

const ADMIN_HTML: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>m7s-rs Admin</title>
<style>body{font:14px system-ui;margin:2rem;background:#111;color:#eee}table{border-collapse:collapse;width:100%}th,td{border:1px solid #333;padding:.5rem;text-align:left}code{color:#7cf}</style></head>
<body><h1>m7s-rs Admin</h1>
<p>Token <input id="token" size="36" placeholder="API token or JWT"> <button onclick="saveToken()">Save</button> <a href="/player" style="color:#7cf">Player</a></p>
<p>Streams: <span id="count">-</span></p><table><thead><tr><th>App</th><th>Stream</th><th>Subscribers</th><th>Bytes</th><th>Started</th><th></th></tr></thead><tbody id="rows"></tbody></table>
<h2>Plugins</h2><table><thead><tr><th>Name</th><th>Status</th><th></th></tr></thead><tbody id="plugs"></tbody></table>
<script>
const tok=()=>localStorage.getItem('m7s_token')||'';
const hdr=()=>tok()?{'Authorization':'Bearer '+tok()}:{};
function saveToken(){localStorage.setItem('m7s_token',document.getElementById('token').value);load();loadPlugins();}
async function load(){
 const r=await fetch('/api/streams',{headers:hdr()}); const j=await r.json();
 const list=(j.data&&j.data.streams)||[]; document.getElementById('count').textContent=list.length;
 document.getElementById('rows').innerHTML=list.map(s=>`<tr><td>${s.app}</td><td>${s.name}</td><td>${s.subscribers}</td><td>${s.bytes}</td><td>${s.started}</td><td><button onclick="del('${s.key}')">Delete</button></td></tr>`).join('');
}
async function loadPlugins(){
 const r=await fetch('/api/plugins',{headers:hdr()}); const j=await r.json();
 const list=(j.plugins)||[];
 document.getElementById('plugs').innerHTML=list.map(p=>`<tr><td>${p.name}</td><td>${p.status}</td><td><button onclick="toggle('${p.name}',${p.status!=='enabled'})">${p.status==='enabled'?'Disable':'Enable'}</button></td></tr>`).join('');
}
async function toggle(n,on){await fetch('/api/plugins/toggle?name='+encodeURIComponent(n)+'&enabled='+(on?1:0),{headers:hdr()});loadPlugins();}
async function del(k){await fetch('/api/stream/delete?stream='+encodeURIComponent(k),{headers:hdr()});load();}
document.getElementById('token').value=tok(); load(); loadPlugins(); setInterval(load,3000);
</script></body></html>"#;

const SDK_JS: &str = include_str!("../web/sdk.js");
const ADMIN_REACT_JS: &str = include_str!("../web/admin.js");
const ADMIN_REACT_HTML: &str = "<!doctype html><html><body><div id=\"root\"></div><script src=\"/admin.js\"></script></body></html>";
const PLAYER_HTML: &str = include_str!("../web/player.html");


fn hls_master(state: &AppState, app: &str, name: &str) -> Option<String> {
    let streams = state.streams.list();
    let has = streams.iter().any(|s| {
        s["app"].as_str() == Some(app) && s["name"].as_str() == Some(name)
    });
    if !has {
        return None;
    }
    let mut variants: Vec<(String, u32)> = vec![(name.to_string(), hls_bandwidth(name).unwrap_or(800))];
    for s in streams.iter() {
        if s["app"].as_str() != Some(app) {
            continue;
        }
        let Some(variant) = s["name"].as_str() else {
            continue;
        };
        if variant == name {
            continue;
        }
        let Some(rest) = variant.strip_prefix(&format!("{name}_")) else {
            continue;
        };
        let Some(kbps) = rest.parse::<u32>().ok().filter(|v| *v >= 100) else {
            continue;
        };
        variants.push((variant.to_string(), kbps));
    }
    variants.sort_by_key(|(_, kbps)| *kbps);
    variants.dedup_by(|a, b| a.1 == b.1);
    let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-INDEPENDENT-SEGMENTS\n");
    for (variant, kbps) in variants {
        out.push_str(&format!(
            "#EXT-X-STREAM-INF:BANDWIDTH={},CODECS=\"avc1.42e01e,mp4a.40.2\"\n../{variant}/index.m3u8\n",
            kbps.saturating_mul(1000)
        ));
    }
    Some(out)
}

fn hls_bandwidth(name: &str) -> Option<u32> {
    name.rsplit_once('_')
        .and_then(|(_, suffix)| suffix.parse::<u32>().ok())
        .filter(|v| *v >= 100)
}

fn fmp4_segment(data: &[u8], seq: u64, enabled: bool, key: &[u8; 16]) -> Vec<u8> {
    if enabled {
        crate::hls::encrypt_segment(data, key, seq)
    } else {
        data.to_vec()
    }
}

pub(crate) async fn respond_redirect<S>(socket: &mut S, location: &str) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let head = format!(
        "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n"
    );
    socket.write_all(head.as_bytes()).await?;
    socket.flush().await?;
    let _ = socket.shutdown().await;
    Ok(())
}

fn parse_range(value: &str) -> Option<(usize, usize)> {
    let (start, end) = value.split_once('-')?;
    let start: usize = start.trim().parse().ok()?;
    let end = end.trim();
    if end.is_empty() {
        Some((start, usize::MAX))
    } else {
        Some((start, end.parse().ok()?))
    }
}

async fn respond_bytes_range<S>(
    socket: &mut S,
    content_type: &str,
    body: &[u8],
    range: Option<(usize, usize)>,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let Some((start, requested_end)) = range else {
        return respond_bytes(socket, 200, content_type, body).await;
    };
    if start >= body.len() {
        return respond_text(socket, 416, "text/plain", "range not satisfiable").await;
    }
    let end = requested_end.min(body.len() - 1);
    if end < start {
        return respond_text(socket, 416, "text/plain", "bad range").await;
    }
    let chunk = &body[start..=end];
    let head = format!(
        "HTTP/1.1 206 Partial Content\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        chunk.len(),
        body.len()
    );
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(chunk).await?;
    socket.flush().await?;
    let _ = socket.shutdown().await;
    Ok(())
}

fn fmp4_playlist_key(mut playlist: String) -> String {
    let map = "#EXT-X-MAP:URI=\"init.mp4\"";
    if playlist.contains(map) {
        playlist = playlist.replacen(
            map,
            "#EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"",
            1,
        );
    }
    playlist
}
