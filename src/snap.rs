//! Snapshot endpoint adapter.

use std::path::PathBuf;

pub async fn capture(app: &str, name: &str) -> Result<PathBuf, String> {
    let url = format!("rtsp://127.0.0.1:8554/{app}/{name}");
    let path = PathBuf::from(format!("data/snapshot/{app}_{name}.jpg"));
    let out = path.clone();
    tokio::task::spawn_blocking(move || m7s_plugin_snap::capture(&url, &out))
        .await
        .map_err(|e| e.to_string())??;
    if !m7s_plugin_snap::is_jpeg(&path) {
        return Err("snapshot is not JPEG".to_string());
    }
    Ok(path)
}
