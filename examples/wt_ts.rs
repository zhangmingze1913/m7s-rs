use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use web_transport_quinn::proto::ConnectRequest;

fn main() -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run())
}

async fn run() -> Result<()> {
    let client = web_transport_quinn::ClientBuilder::new()
        .dangerous()
        .with_no_certificate_verification()?;
    let url = url::Url::parse("https://127.0.0.1:4443/live/wt")?;
    let session = client
        .connect(ConnectRequest::new(url))
        .await
        .context("WebTransport connect")?;
    let ts = std::fs::read("tools/test.ts").context("read TS")?;
    let mut sent = 0usize;
    for chunk in ts.chunks(1200) {
        session.send_datagram(Bytes::copy_from_slice(chunk))?;
        sent += chunk.len();
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    println!("SENT {sent} bytes");
    tokio::time::sleep(Duration::from_secs(2)).await;
    Ok(())
}
