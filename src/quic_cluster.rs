//! QUIC cluster relay (origin side) on UDP 9443.
//!
//! Edge opens a bidirectional QUIC stream, writes `app/name\n`, then receives
//! FLV tags framed as: type u8 | timestamp u32 | len u32 | data.

use std::fs;
use std::io;
use std::sync::Arc;

use anyhow::{Context, Result};
use rustls::pki_types::CertificateDer;

use crate::flv::flv_header;
use crate::stream::StreamManager;

pub async fn run(streams: Arc<StreamManager>, advertise: String) -> Result<()> {
    let bind = std::env::var("M7S_QUIC_BIND").unwrap_or_else(|_| "0.0.0.0:9443".to_string());
    run_on(&bind, streams, advertise).await
}

/// Run the QUIC origin on an explicit bind address.
///
/// Tests and multi-origin deployments use distinct loopback addresses / ports;
/// the default `run` keeps the V6-compatible `0.0.0.0:9443` endpoint.
pub async fn run_on(bind: &str, streams: Arc<StreamManager>, advertise: String) -> Result<()> {
    let mut chain = io::BufReader::new(fs::File::open("tests/fixtures/server.pem")?);
    let chain: Vec<CertificateDer> = rustls_pemfile::certs(&mut chain).collect::<Result<_, _>>()?;
    let mut key_reader = io::BufReader::new(fs::File::open("tests/fixtures/server-key.pem")?);
    let key = rustls_pemfile::private_key(&mut key_reader)?.context("missing cluster key")?;
    let server_config = quinn::ServerConfig::with_single_cert(chain, key)?;
    let endpoint = quinn::Endpoint::server(server_config, bind.parse()?)?;
    log::info!("QUIC cluster origin listening on {}", endpoint.local_addr()?);

    while let Some(conn) = endpoint.accept().await {
        let Some(guard) = m7s_sdk::connections::try_acquire() else {
            conn.refuse();
            continue;
        };
        if !crate::plugins::enabled("cluster-quic") {
            conn.refuse();
            continue;
        }
        let streams = streams.clone();
        let advertise = advertise.clone();
        tokio::spawn(async move {
            let _guard = guard;
            let Ok(conn) = conn.await else { return };
            if let Err(e) = handle(conn, streams, advertise).await {
                log::debug!("QUIC cluster connection: {e:#}");
            }
        });
    }
    Ok(())
}

async fn handle(
    conn: quinn::Connection,
    streams: Arc<StreamManager>,
    advertise: String,
) -> Result<()> {
    loop {
        let (mut send, mut recv) = conn.accept_bi().await?;
        let streams = streams.clone();
        let advertise = advertise.clone();
        tokio::spawn(async move {
            let mut req = Vec::new();
            let mut b = [0u8; 1];
            while recv.read_exact(&mut b).await.is_ok() {
                if b[0] == b'\n' || req.len() > 256 {
                    break;
                }
                req.push(b[0]);
            }
            let path = String::from_utf8_lossy(&req).to_string();
            if path == "@ping" {
                let body = serde_json::to_vec(&serde_json::json!({
                    "server_id": "quic-origin",
                    "status": "ok",
                    "streams": streams.count(),
                    "ts": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0),
                }))
                .unwrap_or_default();
                let _ = send.write_all(&(body.len() as u32).to_be_bytes()).await;
                let _ = send.write_all(&body).await;
                return;
            }
            if path == "@sessions" {
                let sessions: Vec<serde_json::Value> = streams
                    .list()
                    .into_iter()
                    .map(|s| {
                        let app = s["app"].as_str().unwrap_or("live");
                        let name = s["name"].as_str().unwrap_or("test");
                        serde_json::json!({
                            "app": app,
                            "stream": name,
                            "http": format!("http://{advertise}/flv/{app}/{name}.flv"),
                        })
                    })
                    .collect();
                let body = serde_json::to_vec(&serde_json::json!({
                    "server_id": "quic-origin",
                    "sessions": sessions,
                }))
                .unwrap_or_default();
                let _ = send.write_all(&(body.len() as u32).to_be_bytes()).await;
                let _ = send.write_all(&body).await;
                return;
            }
            let (app, name) = path.split_once('/').unwrap_or(("live", "test"));
            let Some(stream) = streams.try_get_or_create(app, name) else {
                return;
            };
            let mut reader = stream.ring_reader();
            if send.write_all(&flv_header(true, true)).await.is_err() {
                stream.leave();
                return;
            }
            loop {
                let mut got = false;
                while let Some(tag) = reader.try_next() {
                    got = true;
                    if write_tag(&mut send, &tag).await.is_err() {
                        stream.leave();
                        return;
                    }
                }
                if !got {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
        });
    }
}

async fn write_tag(send: &mut quinn::SendStream, tag: &crate::flv::FlvTag) -> Result<()> {
    let mut hdr = [0u8; 9];
    hdr[0] = tag.tag_type;
    hdr[1..5].copy_from_slice(&tag.timestamp.to_be_bytes());
    hdr[5..9].copy_from_slice(&(tag.data.len() as u32).to_be_bytes());
    send.write_all(&hdr).await?;
    send.write_all(&tag.data).await?;
    Ok(())
}
