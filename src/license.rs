//! Offline commercial license token.
#![allow(clippy::items_after_test_module)]
//!
//! Token format: base64url(payloadJson).base64url(HMAC-SHA256(payload)).
//! Secret comes from `M7S_LICENSE_SECRET`.
//! Payload: {"edition":"pro","expires":<unix>,"seats":N,"customer":"..."}

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::hmac;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub edition: String,
    pub expires: u64,
    pub seats: u32,
    #[serde(default)]
    pub customer: String,
}

fn enforced() -> bool {
    std::env::var("M7S_ENFORCE_LICENSE").ok().as_deref() == Some("1")
}

fn known_edition(edition: &str) -> bool {
    matches!(edition, "free" | "community" | "pro" | "enterprise")
}

fn current_token() -> Option<String> {
    if let Ok(token) = std::env::var("M7S_LICENSE") {
        if !token.is_empty() {
            return Some(token);
        }
    }
    std::fs::read_to_string(token_file())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn token_file() -> std::path::PathBuf {
    std::env::var("M7S_LICENSE_FILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("data/license.token"))
}

pub fn activate_at(path: &std::path::Path, token: &str) -> Result<Claims, String> {
    let claims = verify(token)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, token).map_err(|e| e.to_string())?;
    Ok(claims)
}

pub fn activate(token: &str) -> Result<Claims, String> {
    let path = token_file();
    let claims = activate_at(&path, token)?;
    unsafe { std::env::set_var("M7S_LICENSE", token) };
    Ok(claims)
}

pub fn revoke() {
    let _ = std::fs::remove_file(token_file());
    unsafe { std::env::remove_var("M7S_LICENSE") };
}

fn active_claims() -> Option<Claims> {
    if !enforced() {
        return None;
    }
    let token = current_token()?;
    let claims = verify(&token).ok()?;
    (claims.expires > now() && known_edition(&claims.edition)).then_some(claims)
}

pub fn premium_allowed() -> bool {
    if !enforced() {
        return true;
    }
    active_claims().is_some_and(|c| matches!(c.edition.as_str(), "pro" | "enterprise"))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn seats_available(used: u32) -> bool {
    if !enforced() {
        return true;
    }
    active_claims().is_some_and(|c| {
        matches!(c.edition.as_str(), "pro" | "enterprise") && used < c.seats
    })
}

pub fn cluster_allowed() -> bool {
    if !enforced() {
        return true;
    }
    active_claims().is_some_and(|c| c.edition == "enterprise")
}

pub fn status(seats_used: u32) -> Value {
    let token = current_token().unwrap_or_default();
    if token.is_empty() {
        return json!({
            "edition":"community",
            "valid":true,
            "reason":"no license token",
            "connection_limit": edition_limit("community"),
            "cluster": cluster_allowed_for("community"),
            "seats_used":seats_used,
            "enforced":std::env::var("M7S_ENFORCE_LICENSE").ok().as_deref() == Some("1"),
        });
    }
    match verify(&token) {
        Ok(claims) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let valid = claims.expires > now;
            json!({
                "edition": claims.edition,
                "valid": valid,
                "expires": claims.expires,
                "seats": claims.seats,
                "seats_used": seats_used,
                "within_seats": seats_used < claims.seats,
                "customer": claims.customer,
                "connection_limit": edition_limit(&claims.edition),
                "cluster": cluster_allowed_for(&claims.edition),
                "reason": if valid { "ok" } else { "expired" },
            })
        }
        Err(e) => json!({"edition":"community","valid":false,"reason":e}),
    }
}

pub fn verify(token: &str) -> Result<Claims, String> {
    let (payload, sig) = token.split_once('.').ok_or("bad token format")?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| "bad payload base64")?;
    let sig = URL_SAFE_NO_PAD.decode(sig).map_err(|_| "bad sig base64")?;
    let secret = std::env::var("M7S_LICENSE_SECRET").map_err(|_| "missing M7S_LICENSE_SECRET")?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    hmac::verify(&key, &payload, &sig).map_err(|_| "bad signature")?;
    serde_json::from_slice(&payload).map_err(|_| "bad claims".to_string())
}

pub fn sign(claims: &Claims) -> Result<String, String> {
    let payload = serde_json::to_vec(claims).map_err(|e| e.to_string())?;
    let secret = std::env::var("M7S_LICENSE_SECRET").map_err(|_| "missing M7S_LICENSE_SECRET")?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let tag = hmac::sign(&key, &payload);
    Ok(format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(&payload),
        URL_SAFE_NO_PAD.encode(tag.as_ref())
    ))
}

/// Official edition connection limits: free 50, pro 1000, enterprise unlimited.
pub fn edition_limit(edition: &str) -> Option<u32> {
    match edition {
        "free" | "community" => Some(50),
        "pro" => Some(1000),
        "enterprise" => None,
        _ => Some(50),
    }
}

pub fn cluster_allowed_for(edition: &str) -> bool {
    edition == "enterprise"
}

#[cfg(test)]
mod edition_tests {
    use super::*;

    #[test]
    fn official_limits() {
        assert_eq!(edition_limit("free"), Some(50));
        assert_eq!(edition_limit("community"), Some(50));
        assert_eq!(edition_limit("pro"), Some(1000));
        assert_eq!(edition_limit("enterprise"), None);
        assert!(!cluster_allowed_for("pro"));
        assert!(cluster_allowed_for("enterprise"));
    }
}

pub async fn activate_remote(server: &str, token: &str) -> Result<Claims, String> {
    let rest = server.strip_prefix("http://").ok_or("only http:// license server")?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let path = format!("/{path}");
    let mut socket = tokio::net::TcpStream::connect(host)
        .await
        .map_err(|e| e.to_string())?;
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{token}",
        token.len()
    );
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    socket.write_all(req.as_bytes()).await.map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    socket.read_to_end(&mut buf).await.map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&buf);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
        return Err(format!("license server: {}", head.lines().next().unwrap_or("")));
    }
    let issued = body.trim();
    let final_token = if issued.is_empty() { token } else { issued };
    activate(final_token)
}
