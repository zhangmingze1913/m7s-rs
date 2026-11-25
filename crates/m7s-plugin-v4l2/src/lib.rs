//! V4L2 fourcc helpers.
pub fn fourcc(s: &str) -> Option<u32> {
    let b = s.as_bytes();
    if b.len() != 4 { return None; }
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
pub fn fourcc_str(v: u32) -> String {
    let b = v.to_le_bytes();
    String::from_utf8_lossy(&b).to_string()
}
#[cfg(test)] mod tests { use super::*; #[test] fn roundtrip() { assert_eq!(fourcc_str(fourcc("YUYV").unwrap()), "YUYV"); } }
