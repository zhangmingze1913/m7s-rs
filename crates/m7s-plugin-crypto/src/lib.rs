//! Signed URL / token crypto plugin.
use ring::hmac;
pub fn sign(secret: &[u8], data: &[u8]) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    hmac::sign(&key, data).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}
pub fn verify(secret: &[u8], data: &[u8], sig: &str) -> bool {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    let Ok(raw) = (0..sig.len()).step_by(2).map(|i| u8::from_str_radix(&sig[i..i+2],16)).collect::<Result<Vec<_>,_>>() else { return false; };
    hmac::verify(&key, data, &raw).is_ok()
}
#[cfg(test)] mod tests { use super::*; #[test] fn roundtrip() { let s=sign(b"k",b"x"); assert!(verify(b"k",b"x",&s)); } }
