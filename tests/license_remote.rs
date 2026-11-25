use m7s_rs::license::{self, Claims};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[test]
fn remote_activation() {
    let dir = std::env::temp_dir().join(format!("m7s-rem-lic-{}", std::process::id()));
    let file = dir.join("license.token");
    unsafe {
        std::env::set_var("M7S_LICENSE_SECRET", "secret");
        std::env::set_var("M7S_LICENSE_FILE", &file);
        std::env::set_var("M7S_ENFORCE_LICENSE", "1");
        std::env::remove_var("M7S_LICENSE");
    }
    let token = license::sign(&Claims {
        edition: "pro".to_string(),
        expires: 4_000_000_000,
        seats: 2,
        customer: "remote".to_string(),
    })
    .unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let body_token = token.clone();
            tokio::spawn(async move {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 8192];
                let _ = s.read(&mut buf).await.unwrap();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body_token.len(),
                    body_token
                );
                s.write_all(resp.as_bytes()).await.unwrap();
            });
            let claims = license::activate_remote(&format!("http://{addr}/activate"), &token)
                .await
                .unwrap();
            assert_eq!(claims.edition, "pro");
            assert!(file.exists());
            assert!(license::premium_allowed());
            license::revoke();
        });
}
