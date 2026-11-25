use std::fs;
use std::io;
use std::sync::Arc;

use anyhow::{Context, Result};
use rustls::RootCertStore;

fn main() -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run())
}

async fn run() -> Result<()> {
    let mut roots = RootCertStore::empty();
    let mut ca = io::BufReader::new(fs::File::open("tests/fixtures/ca.pem")?);
    for cert in rustls_pemfile::certs(&mut ca) {
        roots.add(cert?)?;
    }
    let cfg = quinn::ClientConfig::with_root_certificates(Arc::new(roots))?;
    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(cfg);
    let conn = endpoint
        .connect("127.0.0.1:9443".parse()?, "localhost")?
        .await
        .context("QUIC control connect")?;
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(b"@sessions\n").await?;
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await?;
    let mut body = vec![0u8; u32::from_be_bytes(len) as usize];
    recv.read_exact(&mut body).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    println!(
        "QUIC_CONTROL server={} sessions={}",
        value["server_id"],
        value["sessions"].as_array().map(|a| a.len()).unwrap_or(0)
    );
    Ok(())
}
