//! Transcode adapter.

use std::path::PathBuf;

pub async fn run(app: &str, name: &str, secs: u32, width: u32) -> Result<PathBuf, String> {
    let input = format!("rtsp://127.0.0.1:8554/{app}/{name}");
    let path = PathBuf::from(format!("data/transcode/{app}_{name}.flv"));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let out = path.to_string_lossy().to_string();
    let secs = secs.clamp(1, 60);
    let width = width.clamp(160, 1920);
    tokio::task::spawn_blocking(move || {
        m7s_plugin_transcode::run_secs(&input, &out, width, 800, secs)
    })
    .await
    .map_err(|e| e.to_string())??;
    let ok = std::fs::read(&path)
        .map(|b| b.len() > 3 && &b[0..3] == b"FLV")
        .unwrap_or(false);
    if !ok {
        return Err("transcode output is not FLV".to_string());
    }
    Ok(path)
}
