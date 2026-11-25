//! HomeKit pairing code + mDNS TXT helpers.
pub fn pairing_code(seed: u32) -> String {
    let n = 10000000 + seed % 90000000;
    let s = n.to_string();
    format!("{}-{}-{}", &s[0..3], &s[3..5], &s[5..8])
}
pub fn valid(code: &str) -> bool {
    let d: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    d.len() == 8 && !d.starts_with('0')
}
#[cfg(test)] mod tests { use super::*; #[test] fn code_ok() { let c=pairing_code(1); assert!(valid(&c)); } }
