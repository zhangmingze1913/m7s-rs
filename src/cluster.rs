//! Cluster wrapper over `m7s-plugin-cluster`.

use std::sync::Arc;

use serde_json::Value;

use crate::config::ClusterConfig;
use crate::stream::StreamManager;

pub use m7s_plugin_cluster::RelayInfo;

pub struct Cluster {
    inner: Arc<m7s_plugin_cluster::Cluster>,
    node_id: String,
    advertise: String,
    seeds: Vec<String>,
    shard_redirect: bool,
}

impl Cluster {
    pub fn new(cfg: ClusterConfig, streams: Arc<StreamManager>) -> Arc<Self> {
        let node_id = cfg.node_id.clone();
        let advertise = cfg.advertise.clone();
        let seeds = cfg.seeds.clone();
        let shard_redirect = cfg.shard_redirect;
        let options = m7s_sdk::ClusterOptions {
            enabled: cfg.enabled,
            node_id: cfg.node_id,
            advertise: cfg.advertise,
            seeds: cfg.seeds,
            relay_timeout_secs: cfg.relay_timeout_secs,
        };
        Arc::new(Self {
            inner: m7s_plugin_cluster::Cluster::new(options, streams as Arc<dyn m7s_sdk::StreamApi>),
            node_id,
            advertise,
            shard_redirect,
            seeds,
        })
    }

    pub fn node_id(&self) -> &str {
        self.node_id.as_str()
    }

    pub fn sessions_json(&self) -> Value {
        self.inner.sessions_json()
    }

    pub fn servers_json(&self) -> Value {
        self.inner.servers_json()
    }

    pub fn status_json(&self) -> Value {
        self.inner.status_json()
    }

    pub fn relays_json(&self) -> Value {
        self.inner.relays_json()
    }

    pub async fn ensure_relay(&self, app: &str, name: &str) -> bool {
        self.inner.ensure_relay(app, name).await
    }
}

impl Cluster {
    /// Deterministic stream shard owner: self plus configured seeds.
    pub fn allocate_play(&self, app: &str, name: &str) -> (String, bool, String) {
        let mut nodes = vec![self.node_id.clone()];
        nodes.extend(self.seeds.iter().cloned());
        nodes.sort();
        nodes.dedup();
        let key = format!("{app}/{name}");
        let idx = (fnv1a(&key) as usize) % nodes.len();
        let owner = nodes[idx].clone();
        let redirect = owner != self.node_id;
        let address = if redirect { owner.clone() } else { self.advertise.clone() };
        (owner, redirect, address)
    }
}

fn fnv1a(s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

impl Cluster {
    pub fn redirect_flv(&self, app: &str, name: &str) -> Option<String> {
        if !self.shard_redirect {
            return None;
        }
        let (_, redirect, address) = self.allocate_play(app, name);
        redirect.then(|| format!("http://{address}/flv/{app}/{name}.flv"))
    }

    pub fn redirect_hls(&self, app: &str, name: &str) -> Option<String> {
        if !self.shard_redirect {
            return None;
        }
        let (_, redirect, address) = self.allocate_play(app, name);
        redirect.then(|| format!("http://{address}/hls/{app}/{name}/index.m3u8"))
    }
}
