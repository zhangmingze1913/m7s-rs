//! Minimal HS256 JWT for API auth.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::hmac;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: u64,
    #[serde(default = "default_role")]
    pub role: String,
}

fn default_role() -> String {
    "admin".to_string()
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn issue(secret: &str, subject: &str, ttl_secs: u64) -> String {
    issue_with_role(secret, subject, "admin", ttl_secs)
}

pub fn issue_with_role(secret: &str, subject: &str, role: &str, ttl_secs: u64) -> String {
    let claims = Claims {
        sub: subject.to_string(),
        exp: now() + ttl_secs.max(1),
        role: role.to_string(),
    };
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap_or_default());
    let signing = format!("{header}.{payload}");
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let sig = URL_SAFE_NO_PAD.encode(hmac::sign(&key, signing.as_bytes()).as_ref());
    format!("{signing}.{sig}")
}

pub fn verify(secret: &str, token: &str) -> Option<Claims> {
    let mut parts = token.split('.');
    let header = parts.next()?;
    let payload = parts.next()?;
    let sig = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let raw_sig = URL_SAFE_NO_PAD.decode(sig).ok()?;
    hmac::verify(&key, format!("{header}.{payload}").as_bytes(), &raw_sig).ok()?;
    let claims: Claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    (claims.exp > now()).then_some(claims)
}

pub fn login_json(secret: &str, password: &str) -> serde_json::Value {
    let expected = std::env::var("M7S_ADMIN_PASSWORD").unwrap_or_default();
    if expected.is_empty() || password != expected {
        return json!({"code":401,"message":"invalid password"});
    }
    json!({"code":0,"token":issue(secret, "admin", 3600)})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let token = issue("secret", "admin", 60);
        let claims = verify("secret", &token).unwrap();
        assert_eq!(claims.sub, "admin");
        assert!(verify("other", &token).is_none());
    }
}
