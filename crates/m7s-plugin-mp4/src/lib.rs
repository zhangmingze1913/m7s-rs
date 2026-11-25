//! MP4 recording plugin (FFmpeg remux).
use std::path::Path;
pub fn record(input: &str, output: &Path, secs: u32) -> Result<(), String> {
    if let Some(d) = output.parent() { std::fs::create_dir_all(d).map_err(|e| e.to_string())?; }
    let ff = std::env::var("M7S_FFMPEG").unwrap_or_else(|_| {
        if Path::new("node_modules/ffmpeg-static/ffmpeg.exe").exists() {
            "node_modules/ffmpeg-static/ffmpeg.exe".into()
        } else {
            "ffmpeg".into()
        }
    });
    let st = std::process::Command::new(ff)
        .args(["-hide_banner","-loglevel","error","-y","-i",input,"-t",&secs.to_string(),"-c","copy"])
        .arg(output).status().map_err(|e| e.to_string())?;
    if st.success() { Ok(()) } else { Err(format!("ffmpeg {st}")) }
}
pub fn is_mp4(path: &Path) -> bool {
    std::fs::read(path).map(|b| b.len() > 8 && &b[4..8] == b"ftyp").unwrap_or(false)
}
