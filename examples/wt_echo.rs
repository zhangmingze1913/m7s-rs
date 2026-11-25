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
    let url = url::Url::parse("https://127.0.0.1:4443")?;
    let session = client
        .connect(ConnectRequest::new(url))
        .await
        .context("WebTransport connect")?;
    session.send_datagram(Bytes::from_static(b"hello-webtransport"))?;
    let reply = session.read_datagram().await.context("read echo")?;
    println!("ECHO {}", String::from_utf8_lossy(&reply));
    session.close(0, b"done");
    Ok(())
}
