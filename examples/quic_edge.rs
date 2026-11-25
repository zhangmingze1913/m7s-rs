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
    let client_config = quinn::ClientConfig::with_root_certificates(Arc::new(roots))?;
    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_config);
    let conn = endpoint
        .connect("127.0.0.1:9443".parse()?, "localhost")?
        .await
        .context("QUIC cluster connect")?;
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(b"live/test\n").await?;
    let mut header = [0u8; 13];
    recv.read_exact(&mut header).await?;
    anyhow::ensure!(&header[..3] == b"FLV", "bad FLV header");
    let mut tags = 0u64;
    let mut bytes = 0u64;
    loop {
        let mut hdr = [0u8; 9];
        if recv.read_exact(&mut hdr).await.is_err() {
            break;
        }
        let len = u32::from_be_bytes(hdr[5..9].try_into().unwrap()) as usize;
        let mut data = vec![0u8; len];
        recv.read_exact(&mut data).await?;
        tags += 1;
        bytes += len as u64;
        if tags >= 50 {
            break;
        }
    }
    println!("QUIC_EDGE tags={tags} bytes={bytes}");
    conn.close(0u32.into(), b"done");
    Ok(())
}
