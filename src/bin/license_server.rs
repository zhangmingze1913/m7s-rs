#[tokio::main]
async fn main() -> std::io::Result<()> {
    let bind = std::env::var("M7S_LICENSE_BIND").unwrap_or_else(|_| "127.0.0.1:8090".to_string());
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    m7s_rs::license_server::run(listener).await
}
