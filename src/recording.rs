//! Recording file listing / safe playback helpers.

use std::path::{Path, PathBuf};

pub fn record_dir() -> PathBuf {
    std::env::var("M7S_RECORD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/record"))
}

pub fn mp4_dir() -> PathBuf {
    std::env::var("M7S_MP4_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/mp4"))
}

pub fn list(dir: &Path, extensions: &[&str]) -> Vec<(String, u64, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, u64, u64)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let ext = path.extension()?.to_str()?.to_ascii_lowercase();
            if !extensions.iter().any(|e| *e == ext) {
                return None;
            }
            let name = path.file_name()?.to_str()?.to_string();
            let size = entry.metadata().ok()?.len();
            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            Some((name, size, modified))
        })
        .collect();
    out.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    out
}

pub fn read(dir: &Path, file: &str) -> Option<Vec<u8>> {
    if file.contains('/') || file.contains('\\') || file.contains("..") {
        return None;
    }
    let root = dir.canonicalize().ok()?;
    let path = root.join(file).canonicalize().ok()?;
    if !path.starts_with(&root) {
        return None;
    }
    std::fs::read(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_reads_safely() {
        let dir = std::env::temp_dir().join(format!("m7s-rec-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("a.flv"), b"flv").unwrap();
        std::fs::write(dir.join("b.txt"), b"txt").unwrap();
        let list = list(&dir, &["flv", "mp4"]);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, "a.flv");
        assert_eq!(read(&dir, "a.flv").as_deref(), Some(&b"flv"[..]));
        assert!(read(&dir, "../a.flv").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
