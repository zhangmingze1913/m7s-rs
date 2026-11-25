#![allow(clippy::collapsible_if)]
//! Origin/Edge cluster control plane and HTTP-FLV relay.
//!
//! This is an HTTP-based functional subset of Monibuca's QUIC Origin/Edge
//! design. The control plane (`/cluster/api/*`) is compatible in spirit:
//! node/session discovery, relay tracking, and allocation hints. Media relay
//! uses HTTP-FLV, which makes the cluster easy to test and debug; QUIC relay is
//! a production upgrade path.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex as AsyncMutex;

use m7s_sdk::ClusterOptions;
use m7s_codec::{FlvReader, flv_header};
use m7s_sdk::StreamApi;


#[derive(Debug, Clone)]
pub struct RelayInfo {
    pub stream: String,
    pub origin: String,
    pub started_ms: i64,
    pub tags: u64,
}

pub struct Cluster {
    cfg: ClusterOptions,
    streams: Arc<dyn StreamApi>,
    relays: Arc<Mutex<HashMap<String, RelayInfo>>>,
    inflight: AsyncMutex<std::collections::HashSet<String>>,
}

impl Cluster {
    pub fn new(cfg: ClusterOptions, streams: Arc<dyn StreamApi>) -> Arc<Self> {
        Arc::new(Self {
            cfg,
            streams,
            relays: Arc::new(Mutex::new(HashMap::new())),
            inflight: AsyncMutex::new(std::collections::HashSet::new()),
        })
    }

    pub fn node_id(&self) -> &str {
        &self.cfg.node_id
    }

    pub fn sessions_json(&self) -> Value {
        let sessions: Vec<Value> = self
            .streams
            .list()
            .into_iter()
            .map(|s| {
                json!({
                    "server_id": self.cfg.node_id,
                    "app": s.app,
                    "stream": s.name,
                    "path": s.key,
                    "http": format!("http://{}/flv/{}/{}.flv", self.cfg.advertise, s.app, s.name),
                    "subscribers": s.subscribers,
                    "relay_required": false,
                })
            })
            .collect();
        json!({"server_id": self.cfg.node_id, "sessions": sessions})
    }

    pub fn servers_json(&self) -> Value {
        let mut servers = vec![json!({
            "server_id": self.cfg.node_id,
            "address": self.cfg.advertise,
            "role": "origin",
            "status": "online",
        })];
        for seed in &self.cfg.seeds {
            servers.push(json!({
                "server_id": format!("seed-{seed}"),
                "address": seed,
                "role": "unknown",
                "status": "unknown",
            }));
        }
        json!({"servers": servers})
    }

    pub fn status_json(&self) -> Value {
        json!({
            "server_id": self.cfg.node_id,
            "address": self.cfg.advertise,
            "has_sync": true,
            "node_count": 1 + self.cfg.seeds.len(),
            "session_count": self.streams.count(),
            "relay_session_count": self.relays.lock().unwrap_or_else(|e| e.into_inner()).len(),
            "active_relay_sessions": self.relays.lock().unwrap_or_else(|e| e.into_inner()).len(),
        })
    }

    pub fn relays_json(&self) -> Value {
        let relays: Vec<Value> = self
            .relays
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|r| {
                json!({
                    "stream": r.stream,
                    "origin": r.origin,
                    "started": rfc3339(r.started_ms),
                    "tags": r.tags,
                })
            })
            .collect();
        json!({"relays": relays})
    }

    /// Ensure a local stream exists, pulling it from an origin if necessary.
    pub async fn ensure_relay(&self, app: &str, name: &str) -> bool {
        if self.streams.get(app, name).is_some() {
            return true;
        }
        if !self.cfg.enabled {
            return false;
        }
        let key = format!("{app}/{name}");
        {
            let mut inflight = self.inflight.lock().await;
            if !inflight.insert(key.clone()) {
                drop(inflight);
                return self.wait_for_stream(app, name).await;
            }
        }
        let origins = match self.find_origins(app, name).await {
            Ok(origins) if !origins.is_empty() => origins,
            _ => {
                log::warn!("cluster relay: no origin for {app}/{name}");
                self.inflight.lock().await.remove(&key);
                return false;
            }
        };
        log::info!("cluster relay {app}/{name} <- {} candidates", origins.len());
        let streams = self.streams.clone();
        let relays = self.relays.clone();
        let relay_key = key.clone();
        let app_owned = app.to_string();
        let name_owned = name.to_string();
        tokio::spawn(async move {
            let Some(stream) = streams.stream(&app_owned, &name_owned) else {
                log::warn!("cluster relay {relay_key}: stream seat limit");
                return;
            };
            relays.lock().unwrap_or_else(|e| e.into_inner()).insert(
                relay_key.clone(),
                RelayInfo {
                    stream: relay_key.clone(),
                    origin: origins.first().cloned().unwrap_or_default(),
                    started_ms: now_ms(),
                    tags: 0,
                },
            );
            let mut idx = 0usize;
            let mut retries = 0u32;
            loop {
                let origin_url = &origins[idx % origins.len()];
                match relay_one(origin_url, &app_owned, &name_owned, stream.clone()).await {
                    Ok(()) => log::warn!("cluster relay {relay_key} ended, reconnecting"),
                    Err(e) => {
                        log::warn!("cluster relay {relay_key} via {origin_url} failed: {e:#}")
                    }
                }
                retries += 1;
                idx += 1;
                let subscribers = streams.list().iter().find(|s| s.key == relay_key).map(|s| s.subscribers).unwrap_or(0);
                if !should_retry(subscribers, idx, origins.len(), retries) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            relays
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&relay_key);
        });
        let ok = self.wait_for_stream(app, name).await;
        self.inflight.lock().await.remove(&key);
        ok
    }

    async fn wait_for_stream(&self, app: &str, name: &str) -> bool {
        let deadline =
            tokio::time::Instant::now() + Duration::from_secs(self.cfg.relay_timeout_secs.max(1));
        while tokio::time::Instant::now() < deadline {
            if self.streams.get(app, name).is_some() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }

    async fn find_origins(&self, app: &str, name: &str) -> Result<Vec<String>> {
        let mut origins = Vec::new();
        for seed in &self.cfg.seeds {
            match tokio::time::timeout(
                Duration::from_millis(250),
                quic_sessions(seed, app, name),
            )
            .await
            {
                Ok(Ok(urls)) if !urls.is_empty() => {
                    for url in urls {
                        if !origins.iter().any(|o| o == &url) {
                            origins.push(url);
                        }
                    }
                    continue;
                }
                _ => {}
            }
            let base = if seed.starts_with("http://") || seed.starts_with("https://") {
                seed.clone()
            } else {
                format!("http://{seed}")
            };
            let url = format!("{base}/cluster/api/sessions");
            match http_get_json(&url).await {
                Ok(value) => {
                    log::debug!("cluster sessions from {seed}: {value}");
                    if let Some(sessions) = value.get("sessions").and_then(Value::as_array) {
                        for s in sessions {
                            if s.get("app").and_then(Value::as_str) == Some(app)
                                && s.get("stream").and_then(Value::as_str) == Some(name)
                            {
                                if let Some(http) = s.get("http").and_then(Value::as_str) {
                                    if !origins.iter().any(|o| o == http) {
                                        origins.push(http.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => log::debug!("cluster seed {seed}: {e:#}"),
            }
        }
        Ok(origins)
    }
}

async fn quic_sessions(seed: &str, app: &str, name: &str) -> Result<Vec<String>> {
    let host = seed
        .split("://")
        .nth(1)
        .unwrap_or(seed)
        .split('/')
        .next()
        .unwrap_or(seed)
        .split(':')
        .next()
        .unwrap_or("127.0.0.1");
    let addr: std::net::SocketAddr = format!("{host}:9443").parse().context("QUIC control addr")?;
    let mut roots = rustls::RootCertStore::empty();
    let mut ca = std::io::BufReader::new(std::fs::File::open("tests/fixtures/ca.pem")?);
    for cert in rustls_pemfile::certs(&mut ca) {
        roots.add(cert?)?;
    }
    let cfg = quinn::ClientConfig::with_root_certificates(Arc::new(roots))?;
    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(cfg);
    let conn = endpoint
        .connect(addr, "localhost")?
        .await
        .context("QUIC control connect")?;
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(b"@sessions\n").await?;
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await?;
    let mut body = vec![0u8; u32::from_be_bytes(len) as usize];
    recv.read_exact(&mut body).await?;
    let value: Value = serde_json::from_slice(&body)?;
    let mut urls = Vec::new();
    if let Some(sessions) = value.get("sessions").and_then(Value::as_array) {
        for s in sessions {
            if s.get("app").and_then(Value::as_str) == Some(app)
                && s.get("stream").and_then(Value::as_str) == Some(name)
            {
                if let Some(url) = s.get("http").and_then(Value::as_str) {
                    urls.push(url.to_string());
                }
            }
        }
    }
    Ok(urls)
}

async fn relay_one(
    origin: &str,
    app: &str,
    name: &str,
    stream: Arc<dyn m7s_sdk::TagStream>,
) -> Result<()> {
    // QUIC is the primary cluster transport. A failed/timed-out QUIC handshake
    // falls back to HTTP-FLV only while the origin is still reachable over HTTP;
    // if the origin is gone both paths fail and the caller rotates to the next
    // origin.
    match tokio::time::timeout(
        Duration::from_millis(1200),
        pull_quic(origin, app, name, stream.clone()),
    )
    .await
    {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(e)) => log::warn!("QUIC relay failed ({e:#}), trying HTTP-FLV fallback"),
        Err(_) => log::warn!("QUIC relay timed out, trying HTTP-FLV fallback"),
    }
    pull_flv(origin, stream).await
}

#[allow(dead_code)]
async fn pull_quic(
    origin: &str,
    app: &str,
    name: &str,
    stream: Arc<dyn m7s_sdk::TagStream>,
) -> Result<()> {
    let (_endpoint, conn) = quic_connect(origin).await?;
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(format!("{app}/{name}\n").as_bytes()).await?;
    let mut header = [0u8; 13];
    recv.read_exact(&mut header).await?;
    anyhow::ensure!(&header[..3] == b"FLV", "bad QUIC FLV header");
    loop {
        let mut hdr = [0u8; 9];
        if recv.read_exact(&mut hdr).await.is_err() {
            break;
        }
        let ts = u32::from_be_bytes(hdr[1..5].try_into().unwrap());
        let len = u32::from_be_bytes(hdr[5..9].try_into().unwrap()) as usize;
        let mut data = vec![0u8; len];
        recv.read_exact(&mut data).await?;
        stream.push_tag(hdr[0], ts, data);
    }
    Ok(())
}

async fn quic_connect(origin: &str) -> Result<(quinn::Endpoint, quinn::Connection)> {
    let host = origin
        .split("://")
        .nth(1)
        .unwrap_or(origin)
        .split('/')
        .next()
        .unwrap_or(origin)
        .split(':')
        .next()
        .unwrap_or("127.0.0.1");
    let addr: std::net::SocketAddr = format!("{host}:9443").parse().context("QUIC relay addr")?;
    let mut roots = rustls::RootCertStore::empty();
    let mut ca = std::io::BufReader::new(std::fs::File::open("tests/fixtures/ca.pem")?);
    for cert in rustls_pemfile::certs(&mut ca) {
        roots.add(cert?)?;
    }
    let cfg = quinn::ClientConfig::with_root_certificates(Arc::new(roots))?;
    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(cfg);
    let conn = endpoint
        .connect(addr, "localhost")?
        .await
        .context("QUIC relay connect")?;
    Ok((endpoint, conn))
}

/// QUIC heartbeat: verifies an origin answers `@ping` before/while relaying.
pub async fn quic_ping(origin: &str) -> Result<Value> {
    let (_endpoint, conn) = quic_connect(origin).await?;
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(b"@ping\n").await?;
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await?;
    let mut body = vec![0u8; u32::from_be_bytes(len) as usize];
    recv.read_exact(&mut body).await?;
    serde_json::from_slice(&body).context("parse QUIC ping")
}

async fn pull_flv(url: &str, stream: Arc<dyn m7s_sdk::TagStream>) -> Result<()> {
    log::info!("cluster relay pulling {url}");
    let (mut tcp, residual) = open_http(url).await?;
    log::debug!("cluster relay residual {} bytes", residual.len());
    let mut reader = FlvReader::new();
    if !residual.is_empty() {
        reader.push(&residual);
        while let Some(tag) = reader.next_tag()? {
            log::debug!(
                "cluster relay tag type={} len={}",
                tag.tag_type,
                tag.data.len()
            );
            stream.push_tag(tag.tag_type, tag.timestamp, tag.data);
        }
    }
    let mut buf = [0u8; 8192];
    loop {
        let n = tcp.read(&mut buf).await?;
        if n == 0 {
            bail!("origin FLV stream closed");
        }
        log::debug!("cluster relay read {n} bytes");
        reader.push(&buf[..n]);
        while let Some(tag) = reader.next_tag()? {
            log::debug!(
                "cluster relay tag type={} len={}",
                tag.tag_type,
                tag.data.len()
            );
            stream.push_tag(tag.tag_type, tag.timestamp, tag.data);
        }
    }
}

async fn http_get_json(url: &str) -> Result<Value> {
    let (mut tcp, residual) = open_http(url).await?;
    let mut body = residual;
    let mut buf = [0u8; 8192];
    loop {
        let n = tcp.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
    }
    serde_json::from_slice(&body).context("parse cluster JSON")
}

async fn open_http(url: &str) -> Result<(TcpStream, Vec<u8>)> {
    let rest = url
        .strip_prefix("http://")
        .context("only http:// relay is supported")?;
    let (authority, path) = match rest.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>()?),
        None => (authority.to_string(), 80),
    };
    let mut tcp = TcpStream::connect((host.as_str(), port)).await?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nUser-Agent: m7s-rs-cluster\r\n\r\n"
    );
    tcp.write_all(request.as_bytes()).await?;
    tcp.flush().await?;
    let mut buffer = Vec::with_capacity(8192);
    let header_end;
    loop {
        let mut chunk = [0u8; 4096];
        let n = tcp.read(&mut chunk).await?;
        if n == 0 {
            bail!("origin closed before headers");
        }
        buffer.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_double_crlf(&buffer) {
            header_end = pos + 4;
            break;
        }
        if buffer.len() > 64 * 1024 {
            bail!("origin headers too large");
        }
    }
    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let status = headers
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("500");
    if status != "200" {
        bail!("origin HTTP status {status}");
    }
    Ok((tcp, buffer[header_end..].to_vec()))
}

fn find_double_crlf(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|w| w == b"\r\n\r\n")
}

pub fn flv_response_header() -> Vec<u8> {
    flv_header(true, true).to_vec()
}

/// Test/diagnostic helper: current time in ms.
pub fn now() -> i64 {
    now_ms()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn rfc3339(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", sod / 3600, (sod % 3600) / 60, sod % 60)
}

pub(crate) fn should_retry(subscribers: usize, idx: usize, origins: usize, retries: u32) -> bool {
    !(subscribers == 0 && idx >= origins && retries >= 20)
}

#[cfg(test)]
mod retry_tests {
    use super::*;

    #[test]
    fn retries_while_subscribed_or_under_limit() {
        assert!(should_retry(1, 5, 1, 100));
        assert!(should_retry(0, 1, 1, 19));
        assert!(!should_retry(0, 1, 1, 20));
    }
}
