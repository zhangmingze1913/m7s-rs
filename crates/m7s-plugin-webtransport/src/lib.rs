//! WebTransport media ingest (QUIC/HTTP3) on UDP 4443.
//!
//! Clients send MPEG-TS bytes as WebTransport datagrams; the server demuxes
//! H.264/AAC into the shared stream and echoes each datagram.

use std::fs;
use std::io;
use std::sync::Arc;

use anyhow::{Context, Result};
use rustls::pki_types::CertificateDer;
use web_transport_quinn::ServerBuilder;
use web_transport_quinn::proto::ConnectResponse;

use m7s_plugin_hls::Segmenter;
use m7s_plugin_srt::TsDemux;
use m7s_sdk::{StreamApi, TagStream};

pub async fn run_with_context(addr: &str, ctx: m7s_sdk::EngineContext) -> Result<()> {
    let streams = ctx.stream_api().ok_or_else(|| anyhow::anyhow!("EngineContext missing streams"))?;
    run(addr, streams).await
}

pub async fn run(addr: &str, streams: Arc<dyn StreamApi>) -> Result<()> {
    let cert_path = std::env::var("M7S_WEBTRANSPORT_CERT")
        .unwrap_or_else(|_| "tests/fixtures/server.pem".into());
    let key_path = std::env::var("M7S_WEBTRANSPORT_KEY")
        .unwrap_or_else(|_| "tests/fixtures/server-key.pem".into());

    let mut chain = io::BufReader::new(fs::File::open(&cert_path).context("open WT cert")?);
    let chain: Vec<CertificateDer> = rustls_pemfile::certs(&mut chain)
        .collect::<Result<_, _>>()
        .context("load WT certs")?;
    anyhow::ensure!(!chain.is_empty(), "no WebTransport certificate");

    let mut key_reader = io::BufReader::new(fs::File::open(&key_path).context("open WT key")?);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .context("load WT key")?
        .context("missing WT key")?;

    let mut server = ServerBuilder::new()
        .with_addr(addr.parse()?)
        .with_certificate(chain, key)?;
    log::info!("WebTransport listening on {addr}");

    while let Some(request) = server.accept().await {
        let Some(guard) = m7s_sdk::connections::try_acquire() else {
            continue;
        };
        let streams = streams.clone();
        tokio::spawn(async move {
            let _guard = guard;
            if let Err(e) = handle(request, streams).await {
                log::debug!("WebTransport session: {e:#}");
            }
        });
    }
    Ok(())
}

async fn handle(request: web_transport_quinn::Request, streams: Arc<dyn StreamApi>) -> Result<()> {
    let path = request.url.path().to_string();
    let rest = path.trim_start_matches('/');
    let (app, name) = rest.split_once('/').unwrap_or(("live", "test"));
    let Some(stream) = streams.stream(app, name) else {
        anyhow::bail!("stream seat limit exceeded");
    };
    let demux = Arc::new(std::sync::Mutex::new(TsDemux::new(stream)));

    let session = request
        .respond(ConnectResponse::OK)
        .await
        .context("accept WebTransport")?;
    log::info!("WebTransport session accepted path={path}");

    loop {
        tokio::select! {
            datagram = session.read_datagram() => {
                let Ok(datagram) = datagram else { break };
                demux.lock().unwrap_or_else(|e| e.into_inner()).push(&datagram);
                let _ = session.send_datagram(datagram);
            }
            stream = session.accept_bi() => {
                let Ok((_send, mut recv)) = stream else { break };
                let demux = demux.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    while let Ok(Some(n)) = recv.read(&mut buf).await {
                        demux
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(&buf[..n]);
                    }
                });
            }
        }
    }
    Ok(())
}

/// WebTransport egress: connect as a WebTransport client and send MPEG-TS
/// segments from a local stream as datagrams.
pub async fn push(stream: Arc<dyn TagStream>, target: &str) -> Result<()> {
    let cert_path = std::env::var("M7S_WEBTRANSPORT_CERT")
        .unwrap_or_else(|_| "tests/fixtures/server.pem".into());
    let mut reader = io::BufReader::new(fs::File::open(cert_path).context("open WT cert")?);
    let certs: Vec<CertificateDer> = rustls_pemfile::certs(&mut reader)
        .collect::<Result<_, _>>()
        .context("load WT certs")?;
    let client = web_transport_quinn::ClientBuilder::new()
        .with_server_certificates(certs)
        .context("WT client certs")?;
    let url = url::Url::parse(target).context("WT target url")?;
    let session = client.connect(url).await.context("WT connect")?;

    let mut tags = stream.tag_reader();
    let mut segmenter = Segmenter::new(1);
    let mut buf = [0u8; 4096];
    loop {
        let mut got = false;
        while let Some(tag) = tags.try_next() {
            got = true;
            let mut outputs = Vec::new();
            segmenter.push(&tag, &mut outputs);
            for output in outputs {
                for chunk in output.data.chunks(1200) {
                    session
                        .send_datagram_wait(bytes::Bytes::copy_from_slice(chunk))
                        .await
                        .context("WT send datagram")?;
                }
            }
        }
        if !got {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let _ = &mut buf;
    }
}
