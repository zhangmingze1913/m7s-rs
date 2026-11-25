//! Plugin enable/disable lookup shared by protocol plugins.

fn config_path() -> std::path::PathBuf {
    std::env::var("M7S_PLUGIN_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("data/plugins.json"))
}

fn file_disabled(name: &str) -> bool {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .map(|list| list.iter().any(|n| n.eq_ignore_ascii_case(name)))
        .unwrap_or(false)
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
}
