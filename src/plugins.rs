//! Built-in plugin registry.
//!
//! Monibuca-style plugins are compile-time modules in this build. The list
//! below is the 26-plugin P6 scope; each entry maps to a real module.

use serde_json::{Value, json};

fn config_path() -> std::path::PathBuf {
    std::env::var("M7S_PLUGIN_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("data/plugins.json"))
}

fn load_disabled(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

fn save_disabled(path: &std::path::Path, list: &[String]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(list).unwrap_or_default())
}

pub fn enabled(name: &str) -> bool {
    let disabled = std::env::var("M7S_PLUGINS_DISABLED").unwrap_or_default();
    if load_disabled(&config_path()).iter().any(|n| n.eq_ignore_ascii_case(name)) {
        return false;
    }
    !disabled
        .split(',')
        .map(str::trim)
        .any(|n| !n.is_empty() && n.eq_ignore_ascii_case(name))
}

pub fn set_enabled(name: &str, enabled: bool) -> Result<Vec<String>, String> {
    let path = config_path();
    let mut list = load_disabled(&path);
    list.retain(|n| !n.eq_ignore_ascii_case(name));
    if !enabled {
        list.push(name.to_string());
    }
    save_disabled(&path, &list).map_err(|e| e.to_string())?;
    Ok(list)
}

pub fn list() -> Value {
    let disabled = std::env::var("M7S_PLUGINS_DISABLED").unwrap_or_default();
    let file_disabled = load_disabled(&config_path());
    let is_disabled = |name: &str| {
        file_disabled
            .iter()
            .any(|n| n.eq_ignore_ascii_case(name) || (name.starts_with("onvif") && n.eq_ignore_ascii_case("onvif")))
            || disabled.split(',').map(str::trim).any(|n| {
                !n.is_empty()
                    && (n.eq_ignore_ascii_case(name)
                        || (name.starts_with("onvif") && n.eq_ignore_ascii_case("onvif")))
            })
    };
    let plugins = [
        ("rtmp", "RTMP ingest and chunk/AMF0", "rtmp"),
        ("http-flv", "HTTP-FLV live output", "http"),
        ("hls", "HLS playlist and MPEG-TS segments", "hls"),
        ("rtsp", "RTSP TCP/UDP, RTP/RTCP", "rtsp"),
        ("gb28181", "GB28181 SIP, Catalog, PS/RTP", "gb28181"),
        ("webrtc-whep", "WHEP H.264/PCMU playback", "webrtc"),
        ("webrtc-whip", "WHIP H.264 ingest", "whip"),
        ("srt", "SRT listener, TS demux, AES", "srt"),
        (
            "webtransport",
            "WebTransport QUIC/HTTP3 media",
            "webtransport",
        ),
        ("onvif-device", "ONVIF Device Service", "onvif"),
        ("onvif-media", "ONVIF Media Profiles/StreamUri", "onvif"),
        ("onvif-discovery", "ONVIF WS-Discovery", "onvif"),
        ("cluster-http", "Origin/Edge HTTP control plane", "cluster"),
        ("cluster-quic", "QUIC Origin/Edge relay", "quic_cluster"),
        ("record", "FLV recording", "record"),
        ("snap", "JPEG snapshot via FFmpeg", "m7s-plugin-snap"),
        ("auth", "API token/publish key auth", "http"),
        ("tls", "HTTPS/RTMPS TLS", "tls"),
        ("metrics", "Prometheus-style metrics", "http"),
        ("streams-api", "Stream/session JSON API", "http"),
        ("mpegts-mux", "MPEG-TS muxing", "ts"),
        ("mpegts-demux", "MPEG-TS PAT/PMT/PES demux", "srt"),
        ("rtp-rtcp", "RTP/RTCP packetization and SR/RR", "rtsp"),
        ("flv-mux", "FLV tag muxing", "flv"),
        ("avc-config", "AVC sequence header/SPS-PPS", "rtsp"),
        ("hls-segmenter", "HLS segment cache", "hls"),
    ];
    let mut registry = m7s_sdk::Registry::new();
    for (name, desc, module) in plugins {
        let mut spec = m7s_sdk::PluginSpec::new(name, desc, module);
        spec.enabled = !is_disabled(name);
        registry.register(spec);
    }
    let plugins: Vec<Value> = registry
        .list()
        .iter()
        .map(|spec| {
            json!({
                "name": spec.name,
                "description": spec.description,
                "module": spec.module,
                "status": if spec.enabled { "enabled" } else { "disabled" },
            })
        })
        .collect();
    json!({ "count": plugins.len(), "plugins": plugins })
}

#[cfg(test)]
mod tests {
    #[test]
    fn registry_has_26() {
        let value = super::list();
        assert_eq!(value["count"], 26);
        assert_eq!(value["plugins"].as_array().unwrap().len(), 26);
    }

    #[test]
    fn config_roundtrip() {
        let path = std::path::PathBuf::from(format!("target/m7s-plugins-{}.json", std::process::id()));
        super::save_disabled(&path, &["srt".to_string()]).unwrap();
        assert_eq!(super::load_disabled(&path), vec!["srt".to_string()]);
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod enabled_file_tests {
    use super::*;

    #[test]
    fn toggle_affects_enabled() {
        let path = std::env::temp_dir().join(format!("m7s-plugin-enabled-{}.json", std::process::id()));
        unsafe { std::env::set_var("M7S_PLUGIN_CONFIG", &path) };
        set_enabled("hls", false).unwrap();
        assert!(!enabled("hls"));
        set_enabled("hls", true).unwrap();
        assert!(enabled("hls"));
        unsafe { std::env::remove_var("M7S_PLUGIN_CONFIG") };
        let _ = std::fs::remove_file(path);
    }
}
