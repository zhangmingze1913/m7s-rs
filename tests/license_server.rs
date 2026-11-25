use m7s_rs::license::{self, Claims};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn license_service_activates_and_revokes() {
    let _guard = LOCK.lock().unwrap();
    m7s_rs::license_server::reset_activations();
    m7s_rs::license_server::clear_revoked();
    let path = std::path::PathBuf::from(format!(
        "{}/../../target/lic-revoked-svc-{}.json",
        env!("CARGO_MANIFEST_DIR"),
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    unsafe {
        std::env::set_var("M7S_LICENSE_SECRET", "server-secret");
        std::env::set_var("M7S_LICENSE_REVOKED", &path);
        std::env::remove_var("M7S_LICENSE_MAX_ACTIVATIONS");
    }
    let token = license::sign(&Claims {
        edition: "pro".to_string(),
        expires: 4_000_000_000,
        seats: 1,
        customer: "svc".to_string(),
    })
    .unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(m7s_rs::license_server::run(listener));

            let (status, body) = post(port, "/activate", &token).await;
            assert_eq!(status, 200, "{body}");
            assert_eq!(body, token);

            let (status, _) = post(port, "/revoke", &token).await;
            assert_eq!(status, 200);
            let (status, _) = post(port, "/activate", &token).await;
            assert_eq!(status, 403);
        });
}

async fn post(port: u16, path: &str, body: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(
        format!(
            "POST {path} HTTP/1.1\r\nHost: t\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.trim().to_string()).unwrap_or_default();
    (status, body)
}

#[test]
fn license_service_activation_limit() {
    let _guard = LOCK.lock().unwrap();
    m7s_rs::license_server::reset_activations();
    m7s_rs::license_server::clear_revoked();
    unsafe {
        std::env::set_var("M7S_LICENSE_SECRET", "server-secret");
        std::env::set_var("M7S_LICENSE_MAX_ACTIVATIONS", "1");
    }
    let token = license::sign(&Claims {
        edition: "pro".to_string(),
        expires: 4_000_000_000,
        seats: 1,
        customer: "limit".to_string(),
    })
    .unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(m7s_rs::license_server::run(listener));
            let (status, _) = post(port, "/activate", &token).await;
            assert_eq!(status, 200);
            let (status, _) = post(port, "/activate", &token).await;
            assert_eq!(status, 429);
        });
}
