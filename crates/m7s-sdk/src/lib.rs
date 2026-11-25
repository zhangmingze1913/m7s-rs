//! Plugin SDK surface for m7s.
#![allow(clippy::items_after_test_module)]
pub mod connections;

/// A compile-time plugin implementation.
pub mod plugins;

pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn module(&self) -> &str;
}

#[derive(Debug, Clone)]
pub struct PluginSpec {
    pub name: String,
    pub description: String,
    pub module: String,
    pub enabled: bool,
}

impl PluginSpec {
    pub fn new(name: &str, description: &str, module: &str) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            module: module.to_string(),
            enabled: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct Registry {
    specs: Vec<PluginSpec>,
}

/// Shared services injected into plugins at start time.
#[derive(Clone)]
pub struct EngineContext {
    pub node_id: String,
    pub http_base: String,
    pub rtsp_base: String,
    pub meta: Option<std::sync::Arc<dyn MetaStore>>,
    pub http: Option<std::sync::Arc<dyn HttpClient>>,
    pub events: Option<std::sync::Arc<dyn EventSink>>,
    pub streams: Option<std::sync::Arc<dyn StreamApi>>,
}

impl EngineContext {
    pub fn new(node_id: &str, http_base: &str, rtsp_base: &str) -> Self {
        Self {
            node_id: node_id.to_string(),
            http_base: http_base.to_string(),
            rtsp_base: rtsp_base.to_string(),
            meta: None,
            http: None,
            streams: None,
            events: None,
        }
    }
}

/// Sink for protocol demuxers that emit FLV-shaped tags.
pub trait TagSink: Send + Sync {
    fn push_tag(&self, tag_type: u8, timestamp: u32, data: Vec<u8>);
    fn leave(&self);
}

/// Cluster configuration passed to the cluster plugin.
#[derive(Debug, Clone)]
pub struct ClusterOptions {
    pub enabled: bool,
    pub node_id: String,
    pub advertise: String,
    pub seeds: Vec<String>,
    pub relay_timeout_secs: u64,
}

/// Session metadata for cluster/management plugins.
#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub app: String,
    pub name: String,
    pub key: String,
    pub subscribers: usize,
    pub bytes: u64,
}

/// Stream lookup used by protocol plugins.
pub trait StreamApi: Send + Sync {
    fn stream(&self, app: &str, name: &str) -> Option<std::sync::Arc<dyn TagStream>>;
    fn get(&self, app: &str, name: &str) -> Option<std::sync::Arc<dyn TagStream>>;
    fn list(&self) -> Vec<StreamInfo>;
    fn count(&self) -> usize;
    fn remove(&self, _app: &str, _name: &str) -> bool {
        false
    }
}

/// One media tag emitted by a protocol.
#[derive(Debug, Clone)]
pub struct Tag {
    pub tag_type: u8,
    pub timestamp: u32,
    pub data: Vec<u8>,
}

/// Pull-side reader for protocol plugins (HLS/RTSP/WebRTC egress).
pub trait TagReader: Send {
    fn try_next(&mut self) -> Option<Tag>;
}

pub type SharedTag = std::sync::Arc<Tag>;

/// Sink plus a live tag reader.
pub trait TagStream: TagSink {
    fn tag_reader(&self) -> Box<dyn TagReader>;
    /// Recent GOP/config snapshot for SDP/description generation.
    fn snapshot(&self) -> Vec<Tag>;
    /// Broadcast tap for egress plugins that need the push model.
    fn tap(&self) -> tokio::sync::broadcast::Receiver<SharedTag>;
    /// Optional metadata tag (e.g. RTMP onMetaData).
    fn metadata(&self) -> Option<Tag> {
        None
    }
}

/// Runtime lifecycle for a plugin.
pub trait PluginRuntime {
    fn name(&self) -> &str;
    fn start(&mut self, ctx: &EngineContext) -> Result<(), String>;
    fn stop(&mut self) -> Result<(), String>;
}

#[derive(Default)]
pub struct PluginHost {
    running: std::collections::HashSet<String>,
}

impl PluginHost {
    pub fn start(&mut self, plugin: &mut dyn PluginRuntime, ctx: &EngineContext) -> Result<(), String> {
        plugin.start(ctx)?;
        self.running.insert(plugin.name().to_string());
        Ok(())
    }

    pub fn stop(&mut self, plugin: &mut dyn PluginRuntime) -> Result<(), String> {
        plugin.stop()?;
        self.running.remove(plugin.name());
        Ok(())
    }

    pub fn is_running(&self, name: &str) -> bool {
        self.running.contains(name)
    }

    pub fn running(&self) -> usize {
        self.running.len()
    }
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, spec: PluginSpec) {
        if !self.specs.iter().any(|s| s.name == spec.name) {
            self.specs.push(spec);
        }
    }

    pub fn set_enabled(&mut self, name: &str, enabled: bool) {
        if let Some(spec) = self.specs.iter_mut().find(|s| s.name == name) {
            spec.enabled = enabled;
        }
    }

    pub fn list(&self) -> &[PluginSpec] {
        &self.specs
    }

    pub fn count(&self) -> usize {
        self.specs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_toggle() {
        let mut registry = Registry::new();
        registry.register(PluginSpec::new("rtmp", "RTMP", "rtmp"));
        registry.register(PluginSpec::new("rtmp", "dup", "rtmp"));
        assert_eq!(registry.count(), 1);
        registry.set_enabled("rtmp", false);
        assert!(!registry.list()[0].enabled);
    }

    struct Demo;
    impl PluginRuntime for Demo {
        fn name(&self) -> &str {
            "demo"
        }
        fn start(&mut self, _ctx: &EngineContext) -> Result<(), String> {
            Ok(())
        }
        fn stop(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn host_lifecycle() {
        let ctx = EngineContext::new("n1", "http://127.0.0.1:8180", "rtsp://127.0.0.1:8554");
        let mut host = PluginHost::default();
        let mut plugin = Demo;
        host.start(&mut plugin, &ctx).unwrap();
        assert!(host.is_running("demo"));
        assert_eq!(host.running(), 1);
        host.stop(&mut plugin).unwrap();
        assert!(!host.is_running("demo"));
    }
}

impl EngineContext {
    /// Attach the engine StreamApi so protocol plugins no longer need the
    /// engine's concrete StreamManager type.
    pub fn with_streams(mut self, streams: std::sync::Arc<dyn StreamApi>) -> Self {
        self.streams = Some(streams);
        self
    }

    pub fn stream_api(&self) -> Option<std::sync::Arc<dyn StreamApi>> {
        self.streams.clone()
    }

    pub fn stream(&self, app: &str, name: &str) -> Option<std::sync::Arc<dyn TagStream>> {
        self.streams.as_ref()?.stream(app, name)
    }

    pub fn plugin_enabled(&self, name: &str) -> bool {
        plugins::enabled(name)
    }
}

pub trait EventSink: Send + Sync {
    fn emit(&self, topic: &str, payload: &str);
}

impl EngineContext {
    pub fn with_events(mut self, sink: std::sync::Arc<dyn EventSink>) -> Self {
        self.events = Some(sink);
        self
    }

    pub fn emit(&self, topic: &str, payload: &str) -> bool {
        if let Some(sink) = &self.events {
            sink.emit(topic, payload);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod event_tests {
    use super::*;
    struct S(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
    impl EventSink for S {
        fn emit(&self, topic: &str, payload: &str) {
            self.0.lock().unwrap().push(format!("{topic}:{payload}"));
        }
    }
    #[test]
    fn context_emits() {
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let ctx = EngineContext::new("n", "http://x", "rtsp://x").with_events(std::sync::Arc::new(S(log.clone())));
        assert!(ctx.emit("stream.created", "live/x"));
        assert_eq!(log.lock().unwrap()[0], "stream.created:live/x");
    }
}

pub trait MetaStore: Send + Sync {
    fn put(&self, key: &str, value: &str) -> Result<(), String>;
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

impl EngineContext {
    pub fn with_meta(mut self, meta: std::sync::Arc<dyn MetaStore>) -> Self {
        self.meta = Some(meta);
        self
    }

    pub fn meta(&self) -> Option<&std::sync::Arc<dyn MetaStore>> {
        self.meta.as_ref()
    }
}

pub trait HttpClient: Send + Sync {
    fn get(&self, url: &str) -> Result<Vec<u8>, String>;
}

impl EngineContext {
    pub fn with_http(mut self, client: std::sync::Arc<dyn HttpClient>) -> Self {
        self.http = Some(client);
        self
    }

    pub fn http(&self) -> Option<&std::sync::Arc<dyn HttpClient>> {
        self.http.as_ref()
    }
}
