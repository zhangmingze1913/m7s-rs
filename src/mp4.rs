//! MP4 recording adapter.

use std::path::PathBuf;

pub async fn record(app: &str, name: &str, secs: u32) -> Result<PathBuf, String> {
    let input = format!("rtsp://127.0.0.1:8554/{app}/{name}");
    let path = PathBuf::from(format!("data/mp4/{app}_{name}.mp4"));
    let out = path.clone();
    let secs = secs.clamp(1, 3600);
    tokio::task::spawn_blocking(move || m7s_plugin_mp4::record(&input, &out, secs))
        .await
        .map_err(|e| e.to_string())??;
    if !m7s_plugin_mp4::is_mp4(&path) {
        return Err("recording is not MP4".to_string());
    }
    Ok(path)
}
