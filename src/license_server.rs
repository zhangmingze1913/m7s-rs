//! Minimal license activation/revocation HTTP service.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

static ACTIVATIONS: AtomicUsize = AtomicUsize::new(0);

pub fn reset_activations() {
    ACTIVATIONS.store(0, Ordering::SeqCst);
}

pub fn clear_revoked() {
    revoked().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

fn revoked() -> &'static Mutex<HashSet<String>> {
    static R: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashSet::new()))
}

pub async fn run(listener: TcpListener) -> std::io::Result<()> {
    load_revoked();
    log::info!("license service on {}", listener.local_addr()?);
    loop {
        let (socket, _) = listener.accept().await?;
        tokio::spawn(async move {
            let _ = handle(socket).await;
        });
    }
}

async fn handle(mut socket: TcpStream) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end;
    loop {
        let n = socket.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            head_end = p + 4;
            break;
        }
        if buf.len() > 16384 {
            return Ok(());
        }
    }
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
    let len = head
        .lines()
        .find_map(|l| l.strip_prefix("Content-Length:").or_else(|| l.strip_prefix("content-length:")))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < head_end + len {
        let n = socket.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let body = String::from_utf8_lossy(&buf[head_end..]).trim().to_string();
    let response = match path.as_str() {
        "/activate" => match crate::license::verify(&body) {
            Ok(_claims) => {
                let max = std::env::var("M7S_LICENSE_MAX_ACTIVATIONS")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                let current = ACTIVATIONS.fetch_add(1, Ordering::SeqCst);
                if max > 0 && current >= max {
                    resp(429, "activation limit")
                } else if revoked().lock().unwrap().contains(&body) {
                    resp(403, "revoked")
                } else {
                    resp(200, &body)
                }
            }
            Err(_) => resp(403, "invalid token"),
        },
        "/revoke" => {
            revoked().lock().unwrap().insert(body);
            save_revoked();
            resp(200, "revoked")
        }
        _ => resp(404, "not found"),
    };
    socket.write_all(response.as_bytes()).await?;
    socket.shutdown().await
}

fn resp(code: u16, body: &str) -> String {
    format!(
        "HTTP/1.1 {code} OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn revoked_path() -> std::path::PathBuf {
    std::env::var("M7S_LICENSE_REVOKED")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("data/license-revoked.json"))
}

fn load_revoked() {
    if let Ok(text) = std::fs::read_to_string(revoked_path()) {
        if let Ok(list) = serde_json::from_str::<Vec<String>>(&text) {
            let mut set = revoked().lock().unwrap();
            for token in list {
                set.insert(token);
            }
        }
    }
}

fn save_revoked() {
    let path = revoked_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let list: Vec<String> = revoked().lock().unwrap().iter().cloned().collect();
    let _ = std::fs::write(path, serde_json::to_vec(&list).unwrap_or_default());
}
