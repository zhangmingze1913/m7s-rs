//! Room → Mix orchestration.

use std::path::PathBuf;

pub async fn mix_rooms(room: &str, other: &str, secs: u32) -> Result<PathBuf, String> {
    let a = m7s_plugin_meeting::stream_for(room).ok_or("room has no stream")?;
    let b = m7s_plugin_meeting::stream_for(other).ok_or("other room has no stream")?;
    let base = std::env::var("M7S_RTSP_BASE").unwrap_or_else(|_| "rtsp://127.0.0.1:8554".to_string());
    let source = |v: String| {
        if v.starts_with("lavfi:") || v.starts_with('/') || v.starts_with("file:") {
            v
        } else {
            format!("{}/{}", base.trim_end_matches('/'), v.trim_start_matches('/'))
        }
    };
    let inputs = vec![source(a), source(b)];
    let secs = secs.clamp(1, 60);
    let data = tokio::task::spawn_blocking(move || m7s_plugin_mix::mix_bytes(&inputs, secs))
        .await
        .map_err(|e| e.to_string())??;
    if data.len() < 8 || &data[4..8] != b"ftyp" {
        return Err("mix output is not MP4".to_string());
    }
    let dir = PathBuf::from("data/mix");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("room-{}.mp4", crate::time::now_ms()));
    std::fs::write(&path, data).map_err(|e| e.to_string())?;
    Ok(path)
}
