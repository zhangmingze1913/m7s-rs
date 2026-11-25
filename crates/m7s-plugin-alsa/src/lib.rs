//! ALSA PCM parameter validation.
pub fn valid(rate: u32, channels: u8, format: &str) -> bool {
    (8000..=192000).contains(&rate) && (1..=8).contains(&channels) && matches!(format, "S16LE" | "S24LE" | "S32LE" | "F32LE")
}
#[cfg(test)] mod tests { use super::*; #[test] fn pcm() { assert!(valid(48000,2,"S16LE")); assert!(!valid(1,2,"S16LE")); } }
