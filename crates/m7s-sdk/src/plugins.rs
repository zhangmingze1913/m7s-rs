//! Plugin enable/disable lookup shared by protocol plugins.
//!
//! Lookups run on the accept path (RTMP/RTSP/SRT listeners and HLS handlers),
//! so the JSON config is cached behind an mtime check instead of being read and
//! parsed on every call. The cache is invalidated when the file changes, so a
//! runtime config edit still takes effect without a restart.

use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use std::time::SystemTime;

fn config_path() -> PathBuf {
    std::env::var("M7S_PLUGIN_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/plugins.json"))
}

struct DisabledCache {
    path: PathBuf,
    mtime: Option<SystemTime>,
    list: Vec<String>,
}

static DISABLED: OnceLock<RwLock<DisabledCache>> = OnceLock::new();

fn load(path: &PathBuf) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

fn file_disabled(name: &str) -> bool {
    let path = config_path();
    let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let cache = DISABLED.get_or_init(|| RwLock::new(DisabledCache {
        path: path.clone(),
        mtime,
        list: load(&path),
    }));
    {
        let guard = cache.read().unwrap_or_else(|e| e.into_inner());
        if guard.path == path && guard.mtime == mtime {
            return guard.list.iter().any(|n| n.eq_ignore_ascii_case(name));
        }
    }
    // Config changed (or the path switched, e.g. in tests): reload once.
    let list = load(&path);
    let disabled = list.iter().any(|n| n.eq_ignore_ascii_case(name));
    let mut guard = cache.write().unwrap_or_else(|e| e.into_inner());
    guard.path = path;
    guard.mtime = mtime;
    guard.list = list;
    disabled
}

pub fn enabled(name: &str) -> bool {
    if file_disabled(name) {
        return false;
    }
    let disabled = std::env::var("M7S_PLUGINS_DISABLED").unwrap_or_default();
    !disabled
        .split(',')
        .map(str::trim)
        .any(|n| !n.is_empty() && n.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_disables() {
        let key = format!("m7s-test-{}", std::process::id());
        unsafe { std::env::set_var("M7S_PLUGINS_DISABLED", &key) };
        assert!(!enabled(&key));
        unsafe { std::env::remove_var("M7S_PLUGINS_DISABLED") };
        assert!(enabled(&key));
    }

    #[test]
    fn missing_config_is_enabled() {
        assert!(enabled("definitely-not-disabled"));
    }
}
