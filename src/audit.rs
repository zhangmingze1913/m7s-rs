//! Append-only JSONL audit log for admin actions.
#![allow(clippy::items_after_test_module)]

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub ts: u64,
    pub actor: String,
    pub role: String,
    pub action: String,
    pub target: String,
    pub ok: bool,
}

fn path() -> PathBuf {
    std::env::var("M7S_AUDIT_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/audit.log"))
}

static APPEND_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

pub fn append_at(path: &Path, entry: &AuditEntry) -> std::io::Result<()> {
    let _guard = APPEND_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _file_lock = FileLock::acquire(path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", serde_json::to_string(entry)?)?;
    let max = std::env::var("M7S_AUDIT_MAX").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(10_000);
    if max > 0 {
        let _ = trim_at(path, max);
    }
    Ok(())
}

fn trim_at(path: &Path, max: usize) -> std::io::Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max {
        return Ok(());
    }
    let cut = lines.len() - max;
    let removed = lines[..cut].join("\n") + "\n";
    if std::env::var("M7S_AUDIT_ARCHIVE").as_deref() != Ok("0") {
        rotate_archives(path);
        let archive = format!("{}.1", path.display());
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(archive) {
            let _ = file.write_all(removed.as_bytes());
        }
    }
    let keep = lines[cut..].join("\n") + "\n";
    std::fs::write(path, keep)
}


pub fn recent_at(path: &Path, limit: usize) -> Vec<AuditEntry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut entries: Vec<AuditEntry> = text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    if entries.len() > limit {
        entries.drain(0..entries.len() - limit);
    }
    entries
}

pub fn log(actor: &str, role: &str, action: &str, target: &str, ok: bool) {
    let entry = AuditEntry {
        ts: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        actor: actor.to_string(),
        role: role.to_string(),
        action: action.to_string(),
        target: target.to_string(),
        ok,
    };
    let _ = append_at(&path(), &entry);
}

pub fn recent(limit: usize) -> Vec<AuditEntry> {
    recent_at(&path(), limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_keeps_last() {
        let path = std::env::temp_dir().join(format!("m7s-audit-ret-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        for i in 0..3 {
            let e = AuditEntry {
                ts: i,
                actor: "a".into(),
                role: "admin".into(),
                action: format!("e{i}"),
                target: "t".into(),
                ok: true,
            };
            append_at(&path, &e).unwrap();
        }
        trim_at(&path, 2).unwrap();
        let entries = recent_at(&path, 10);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].action, "e1");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rotates_archive_levels() {
        unsafe {
            std::env::set_var("M7S_AUDIT_ARCHIVE_MAX", "1");
            std::env::set_var("M7S_AUDIT_ARCHIVE_LEVELS", "3");
        }
        let path = std::env::temp_dir().join(format!("m7s-audit-rot-{}.log", std::process::id()));
        let a1 = format!("{}.1", path.display());
        let a2 = format!("{}.2", path.display());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&a1);
        let _ = std::fs::remove_file(&a2);
        std::fs::write(&a1, "{}\n{}\n").unwrap();
        std::fs::write(&path, "{}\n{}\n").unwrap();
        trim_at(&path, 1).unwrap();
        assert!(std::path::Path::new(&a2).exists());
        assert_eq!(std::fs::read_to_string(&a1).unwrap().lines().count(), 1);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&a1);
        let _ = std::fs::remove_file(&a2);
    }

    #[test]
    fn append_and_read() {
        let path = std::env::temp_dir().join(format!("m7s-audit-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let e = AuditEntry {
            ts: 1,
            actor: "admin".into(),
            role: "admin".into(),
            action: "plugin.toggle".into(),
            target: "rtmp".into(),
            ok: true,
        };
        append_at(&path, &e).unwrap();
        let entries = recent_at(&path, 10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "plugin.toggle");
        let _ = std::fs::remove_file(path);
    }
}

pub fn export_csv(limit: usize) -> String {
    let mut out = String::from("ts,actor,role,action,target,ok\n");
    for e in recent(limit) {
        out.push_str(&format!(
            "{},{},{},{},{},{}\n",
            e.ts, e.actor, e.role, e.action, e.target, e.ok
        ));
    }
    out
}

struct FileLock(std::path::PathBuf);

impl FileLock {
    fn acquire(path: &Path) -> Option<Self> {
        let lock = std::path::PathBuf::from(format!("{}.lock", path.display()));
        if let Some(dir) = lock.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        for _ in 0..200 {
            match std::fs::OpenOptions::new().create_new(true).write(true).open(&lock) {
                Ok(_) => return Some(Self(lock)),
                Err(_) => {
                    if let Ok(meta) = std::fs::metadata(&lock) {
                        if let Ok(modified) = meta.modified() {
                            if modified.elapsed().map(|d| d.as_secs() > 10).unwrap_or(false) {
                                let _ = std::fs::remove_file(&lock);
                            }
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
        None
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn rotate_archives(path: &Path) {
    let max = std::env::var("M7S_AUDIT_ARCHIVE_MAX")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100_000);
    let levels = std::env::var("M7S_AUDIT_ARCHIVE_LEVELS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(3)
        .max(1);
    let first = format!("{}.1", path.display());
    let count = std::fs::read_to_string(&first)
        .map(|s| s.lines().count())
        .unwrap_or(0);
    let age = std::fs::metadata(&first)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if !archive_due(count, max, age) {
        return;
    }
    for i in (1..levels).rev() {
        let from = format!("{}.{}", path.display(), i);
        let to = format!("{}.{}", path.display(), i + 1);
        if std::path::Path::new(&from).exists() {
            if i == 1 && std::env::var("M7S_AUDIT_COMPRESS").as_deref() == Ok("1") {
                let _ = compress_file(&from, &to);
            } else {
                let _ = std::fs::rename(from, to);
            }
        }
    }
}

fn archive_due(count: usize, max: usize, age_secs: u64) -> bool {
    if count > max {
        return true;
    }
    let hours = std::env::var("M7S_AUDIT_ROTATE_HOURS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    hours > 0 && age_secs >= hours * 3600
}

fn compress_file(from: &str, to: &str) -> std::io::Result<()> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    let data = std::fs::read(from)?;
    let file = std::fs::File::create(format!("{to}.gz"))?;
    let mut enc = GzEncoder::new(file, Compression::default());
    enc.write_all(&data)?;
    enc.finish()?;
    std::fs::remove_file(from)
}

#[cfg(test)]
mod archive_policy_tests {
    use super::*;

    #[test]
    fn due_by_count_or_age() {
        unsafe {
            std::env::remove_var("M7S_AUDIT_ROTATE_HOURS");
        }
        assert!(archive_due(11, 10, 0));
        assert!(!archive_due(5, 10, 999_999));
        unsafe {
            std::env::set_var("M7S_AUDIT_ROTATE_HOURS", "1");
        }
        assert!(archive_due(5, 10, 3600));
        assert!(!archive_due(5, 10, 10));
        unsafe {
            std::env::remove_var("M7S_AUDIT_ROTATE_HOURS");
        }
    }

    #[test]
    fn compresses_archive() {
        use flate2::read::GzDecoder;
        use std::io::Read;
        let base = std::env::temp_dir().join(format!("m7s-audit-gz-{}", std::process::id()));
        let from = format!("{}.log.1", base.display());
        let to = format!("{}.log.2", base.display());
        let _ = std::fs::remove_file(&from);
        let _ = std::fs::remove_file(format!("{to}.gz"));
        std::fs::write(&from, "a\nb\n").unwrap();
        compress_file(&from, &to).unwrap();
        assert!(!std::path::Path::new(&from).exists());
        let mut out = String::new();
        GzDecoder::new(std::fs::File::open(format!("{to}.gz")).unwrap())
            .read_to_string(&mut out)
            .unwrap();
        assert_eq!(out, "a\nb\n");
        let _ = std::fs::remove_file(format!("{to}.gz"));
    }
}
