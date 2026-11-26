use std::path::Path;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub server: ServerConfig,
    pub tls: TlsConfig,
    pub hls: HlsConfig,
    pub rtsp: RtspConfig,
    pub gb28181: Gb28181Config,
    pub record: RecordConfig,
    pub cluster: ClusterConfig,
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TlsConfig {
    pub enabled: bool,
    pub rtmp: bool,
    /// Serve gRPC over TLS when `tls.enabled` is also true.
    pub grpc: bool,
    pub cert: String,
    pub key: String,
    pub require_client_cert: bool,
    pub client_ca: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HlsConfig {
    pub enabled: bool,
    pub segment_secs: u64,
    pub max_segments: usize,
    pub encrypt: bool,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RecordConfig {
    pub enabled: bool,
    pub dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub http_bind: String,
    pub rtmp_bind: String,
    /// Maximum cached FLV tags per stream (GOP cache).
    pub gop_cache: usize,
    pub backlog: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClusterConfig {
    pub enabled: bool,
    pub node_id: String,
    /// HTTP control-plane address advertised to peers.
    pub advertise: String,
    pub seeds: Vec<String>,
    /// Redirect missing FLV/HLS streams to their shard owner.
    pub shard_redirect: bool,
    pub heartbeat_secs: u64,
    pub relay_timeout_secs: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthConfig {
    /// Empty = accept all publishers. Otherwise `?key=` or `stream?key=`.
    pub publish_key: String,
    /// Empty = no token required for `/api/*` and `/cluster/api/*`.
    pub api_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RtspConfig {
    pub enabled: bool,
    pub bind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Gb28181Config {
    pub enabled: bool,
    pub bind: String,
    pub domain: String,
    pub password: String,
    pub media_port: u16,
}

impl Default for Gb28181Config {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: "0.0.0.0:5060".to_string(),
            domain: "3402000000".to_string(),
            password: String::new(),
            media_port: 30000,
        }
    }
}

impl Default for RtspConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: "0.0.0.0:8554".to_string(),
        }
    }
}

impl Default for HlsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            segment_secs: 4,
            max_segments: 6,
            encrypt: false,
            key: String::new(),
        }
    }
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            rtmp: false,
            grpc: false,
            cert: "config/cert.pem".to_string(),
            key: "config/key.pem".to_string(),
            require_client_cert: false,
            client_ca: "config/client-ca.pem".to_string(),
        }
    }
}

impl Default for RecordConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dir: "../data/record".to_string(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            http_bind: "0.0.0.0:8180".to_string(),
            rtmp_bind: "0.0.0.0:1935".to_string(),
            gop_cache: 500,
            backlog: 4096,
        }
    }
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            node_id: "node-1".to_string(),
            advertise: "127.0.0.1:8180".to_string(),
            seeds: Vec::new(),
            shard_redirect: false,
            heartbeat_secs: 5,
            relay_timeout_secs: 10,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let cfg: Config = m7s_config::load(path)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        if self
            .server
            .http_bind
            .parse::<std::net::SocketAddr>()
            .is_err()
        {
            bail!("server.http_bind must be host:port");
        }
        if self
            .server
            .rtmp_bind
            .parse::<std::net::SocketAddr>()
            .is_err()
        {
            bail!("server.rtmp_bind must be host:port");
        }
        if self.server.gop_cache == 0 {
            bail!("server.gop_cache must be > 0");
        }
        if self.cluster.enabled {
            if self.cluster.node_id.trim().is_empty() {
                bail!("cluster.node_id is required");
            }
            if self
                .cluster
                .advertise
                .parse::<std::net::SocketAddr>()
                .is_err()
            {
                bail!("cluster.advertise must be host:port");
            }
            if self.cluster.heartbeat_secs == 0 {
                bail!("cluster.heartbeat_secs must be > 0");
            }
        }
        Ok(())
    }
}

static RELOAD_PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Remember the loaded config path so the admin API can hot-reload it.
pub fn set_reload_path(path: impl Into<std::path::PathBuf>) {
    let _ = RELOAD_PATH.set(path.into());
}

/// Re-read and validate the startup config file.
pub fn reload() -> Result<Config> {
    let path = RELOAD_PATH
        .get()
        .ok_or_else(|| anyhow::anyhow!("config path not registered"))?;
    Config::load(path)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_valid() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn bad_bind() {
        let cfg = Config {
            server: ServerConfig {
                http_bind: "bad".to_string(),
                ..ServerConfig::default()
            },
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }
}

