use std::time::Duration;

use anyhow::{Context, Result};
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
    let url = url::Url::parse("https://127.0.0.1:4443/live/wtstream")?;
    let session = client
        .connect(ConnectRequest::new(url))
        .await
        .context("WebTransport connect")?;
    let ts = std::fs::read("tools/test.ts").context("read TS")?;
    let (mut send, _recv) = session.open_bi().await?;
    for chunk in ts.chunks(4096) {
        send.write_all(chunk).await?;
    }
    let _ = send.finish();
    println!("SENT_STREAM {}", ts.len());
    tokio::time::sleep(Duration::from_secs(1)).await;
    Ok(())
}
