//! Snapshot plugin: grab one JPEG from a stream URL via FFmpeg.

use std::path::Path;
use std::process::Command;

fn ffmpeg() -> String {
    std::env::var("M7S_FFMPEG").unwrap_or_else(|_| {
        if Path::new("node_modules/ffmpeg-static/ffmpeg.exe").exists() {
            "node_modules/ffmpeg-static/ffmpeg.exe".to_string()
        } else {
            "ffmpeg".to_string()
        }
    })
}

/// Capture the first video frame of `input` into `output`.
pub fn capture(input: &str, output: &Path) -> Result<(), String> {
    if let Some(dir) = output.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut child = Command::new(ffmpeg())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
            input,
            "-t",
            "5",
            "-frames:v",
            "1",
            "-an",
            "-q:v",
            "2",
        ])
        .arg(output)
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    for _ in 0..100 {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("ffmpeg exit {status}")),
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    Err("snapshot timeout".to_string())
}

pub fn is_jpeg(path: &Path) -> bool {
    std::fs::read(path)
        .map(|b| b.len() > 3 && b[0] == 0xFF && b[1] == 0xD8 && b[2] == 0xFF)
        .unwrap_or(false)
}
