//! Sandboxed WASM plugin runtime (wasmi, no WASI/host imports).

use wasmi::{Engine, Instance, Memory, Module, Store, TypedFunc};

#[derive(Debug, Clone, PartialEq)]
pub struct AvFrame {
    pub tag_type: u8,
    pub keyframe: bool,
    pub timestamp: u32,
    pub data: Vec<u8>,
}

impl AvFrame {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![0u8; 12 + self.data.len()];
        out[0] = self.tag_type;
        out[1] = u8::from(self.keyframe);
        out[4..8].copy_from_slice(&self.timestamp.to_le_bytes());
        out[8..12].copy_from_slice(&(self.data.len() as u32).to_le_bytes());
        out[12..].copy_from_slice(&self.data);
        out
    }

    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < 12 {
            return None;
        }
        let len = u32::from_le_bytes(buf[8..12].try_into().ok()?) as usize;
        if buf.len() != 12 + len {
            return None;
        }
        Some(Self {
            tag_type: buf[0],
            keyframe: buf[1] != 0,
            timestamp: u32::from_le_bytes(buf[4..8].try_into().ok()?),
            data: buf[12..].to_vec(),
        })
    }
}
pub struct WasmPlugin {

    store: Store<()>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    transform: TypedFunc<(i32, i32), i64>,
    frame: Option<TypedFunc<(i32, i32, i32, i32), i64>>,
    av: Option<TypedFunc<(i32, i32), i64>>,
    _instance: Instance,
}

impl WasmPlugin {
    /// Load a WASM/WAT module exporting `memory`, `alloc(i32)->i32`,
    /// `transform(i32,i32)->i64` where the result packs `(ptr << 32) | len`.
    pub fn load(bytes: &[u8]) -> Result<Self, String> {
        let mut config = wasmi::Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, bytes).map_err(|e| e.to_string())?;
        let mut store = Store::new(&engine, ());
        let fuel = std::env::var("M7S_WASM_FUEL").ok().and_then(|v| v.parse().ok()).unwrap_or(50_000_000);
        store.set_fuel(fuel).map_err(|e| e.to_string())?;
        let linker = wasmi::Linker::new(&engine);
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| e.to_string())?
            .start(&mut store)
            .map_err(|e| e.to_string())?;
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| "missing memory export".to_string())?;
        let alloc = instance
            .get_typed_func::<i32, i32>(&store, "alloc")
            .map_err(|e| e.to_string())?;
        let transform = instance
            .get_typed_func::<(i32, i32), i64>(&store, "transform")
            .map_err(|e| e.to_string())?;
        let frame = instance
            .get_typed_func::<(i32, i32, i32, i32), i64>(&store, "transform_frame")
            .ok();
        let av = instance
            .get_typed_func::<(i32, i32), i64>(&store, "transform_av")
            .ok();
        Ok(Self {
            store,
            memory,
            alloc,
            transform,
            frame,
            av,
            _instance: instance,
        })
    }

    pub fn has_frame_abi(&self) -> bool {
        self.frame.is_some()
    }

    pub fn process_frame(
        &mut self,
        tag_type: u8,
        timestamp: u32,
        input: &[u8],
    ) -> Result<Option<Vec<u8>>, String> {
        let Some(f) = self.frame else {
            return Ok(None);
        };
        if input.len() > 65536 {
            return Err("input too large".into());
        }
        let ptr = self
            .alloc
            .call(&mut self.store, input.len() as i32)
            .map_err(|e| e.to_string())?;
        self.memory
            .write(&mut self.store, ptr as usize, input)
            .map_err(|e| e.to_string())?;
        let packed = f
            .call(&mut self.store, (tag_type as i32, timestamp as i32, ptr, input.len() as i32))
            .map_err(|e| e.to_string())?;
        let out_ptr = ((packed as u64) >> 32) as usize;
        let out_len = (packed as u64 & 0xffff_ffff) as usize;
        if out_len > 1 << 20 {
            return Err("output too large".into());
        }
        let mut out = vec![0u8; out_len];
        self.memory
            .read(&self.store, out_ptr, &mut out)
            .map_err(|e| e.to_string())?;
        Ok(Some(out))
    }

    pub fn has_av_abi(&self) -> bool {
        self.av.is_some()
    }

    pub fn process_av_frame(&mut self, frame: &AvFrame) -> Result<Option<AvFrame>, String> {
        let Some(f) = self.av else {
            return Ok(None);
        };
        let input = frame.encode();
        if input.len() > 65536 {
            return Err("input too large".into());
        }
        let ptr = self
            .alloc
            .call(&mut self.store, input.len() as i32)
            .map_err(|e| e.to_string())?;
        self.memory
            .write(&mut self.store, ptr as usize, &input)
            .map_err(|e| e.to_string())?;
        let packed = f
            .call(&mut self.store, (ptr, input.len() as i32))
            .map_err(|e| e.to_string())?;
        let out_ptr = ((packed as u64) >> 32) as usize;
        let out_len = (packed as u64 & 0xffff_ffff) as usize;
        if out_len > 1 << 20 {
            return Err("output too large".into());
        }
        let mut out = vec![0u8; out_len];
        self.memory
            .read(&self.store, out_ptr, &mut out)
            .map_err(|e| e.to_string())?;
        Ok(AvFrame::decode(&out))
    }

    pub fn process(&mut self, input: &[u8]) -> Result<Vec<u8>, String> {
        if input.len() > 65536 {
            return Err("input too large".into());
        }
        let ptr = self
            .alloc
            .call(&mut self.store, input.len() as i32)
            .map_err(|e| e.to_string())?;
        if ptr < 0 {
            return Err("alloc returned negative".into());
        }
        self.memory
            .write(&mut self.store, ptr as usize, input)
            .map_err(|e| e.to_string())?;
        let packed = self
            .transform
            .call(&mut self.store, (ptr, input.len() as i32))
            .map_err(|e| e.to_string())?;
        let out_ptr = ((packed as u64) >> 32) as usize;
        let out_len = (packed as u64 & 0xffff_ffff) as usize;
        if out_len > 1 << 20 {
            return Err("output too large".into());
        }
        let mut out = vec![0u8; out_len];
        self.memory
            .read(&self.store, out_ptr, &mut out)
            .map_err(|e| e.to_string())?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: &str = r#"
(module
  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 1024))
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    global.get $next
    local.set $p
    global.get $next
    local.get $n
    i32.add
    global.set $next
    local.get $p)
  (func (export "transform") (param $p i32) (param $n i32) (result i64)
    local.get $p
    i64.extend_i32_u
    i64.const 32
    i64.shl
    local.get $n
    i64.extend_i32_u
    i64.or)
  (func (export "transform_frame") (param $t i32) (param $ts i32) (param $p i32) (param $n i32) (result i64)
    local.get $p i64.extend_i32_u i64.const 32 i64.shl
    local.get $n i64.extend_i32_u i64.or)
  (func (export "transform_av") (param $p i32) (param $n i32) (result i64)
    local.get $p i64.extend_i32_u i64.const 32 i64.shl
    local.get $n i64.extend_i32_u i64.or))
"#;

    #[test]
    fn identity_transform() {
        let mut p = WasmPlugin::load(IDENTITY.as_bytes()).unwrap();
        let out = p.process(b"hello").unwrap();
        assert_eq!(out, b"hello");
        assert_eq!(p.process_frame(9, 123, b"xy").unwrap().unwrap(), b"xy");
        let f = AvFrame { tag_type: 9, keyframe: true, timestamp: 42, data: b"zz".to_vec() };
        assert_eq!(p.process_av_frame(&f).unwrap().unwrap(), f);
    }

    #[test]
    fn rejects_garbage() {
        assert!(WasmPlugin::load(b"not wasm").is_err());
    }
}

pub struct WasmRuntime {
    name: String,
    plugin: WasmPlugin,
}

impl WasmRuntime {
    pub fn load(name: &str, bytes: &[u8]) -> Result<Self, String> {
        Ok(Self {
            name: name.to_string(),
            plugin: WasmPlugin::load(bytes)?,
        })
    }

    pub fn has_frame_abi(&self) -> bool {
        self.plugin.has_frame_abi()
    }

    pub fn process_frame(
        &mut self,
        tag_type: u8,
        timestamp: u32,
        input: &[u8],
    ) -> Result<Option<Vec<u8>>, String> {
        self.plugin.process_frame(tag_type, timestamp, input)
    }

    pub fn process(&mut self, input: &[u8]) -> Result<Vec<u8>, String> {
        self.plugin.process(input)
    }
}

impl m7s_sdk::PluginRuntime for WasmRuntime {
    fn name(&self) -> &str {
        &self.name
    }

    fn start(&mut self, _ctx: &m7s_sdk::EngineContext) -> Result<(), String> {
        Ok(())
    }

    fn stop(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::*;

    const WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 1024))
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    global.get $next local.set $p
    global.get $next local.get $n i32.add global.set $next
    local.get $p)
  (func (export "transform") (param $p i32) (param $n i32) (result i64)
    local.get $p i64.extend_i32_u i64.const 32 i64.shl
    local.get $n i64.extend_i32_u i64.or)
  (func (export "transform_frame") (param $t i32) (param $ts i32) (param $p i32) (param $n i32) (result i64)
    local.get $p i64.extend_i32_u i64.const 32 i64.shl
    local.get $n i64.extend_i32_u i64.or)
  (func (export "transform_av") (param $p i32) (param $n i32) (result i64)
    local.get $p i64.extend_i32_u i64.const 32 i64.shl
    local.get $n i64.extend_i32_u i64.or))
"#;

    #[test]
    fn runtime_lifecycle_and_transform() {
        use m7s_sdk::PluginRuntime;
        let mut rt = WasmRuntime::load("identity", WAT.as_bytes()).unwrap();
        let ctx = m7s_sdk::EngineContext::new("n", "http://x", "rtsp://x");
        rt.start(&ctx).unwrap();
        assert_eq!(rt.process(b"abc").unwrap(), b"abc");
        rt.stop().unwrap();
        let mut host = m7s_sdk::PluginHost::default();
        host.start(&mut rt, &ctx).unwrap();
        host.stop(&mut rt).unwrap();
    }

}

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

type WasmEntry = (Vec<u8>, std::path::PathBuf);
type WasmMap = HashMap<String, WasmEntry>;

static PLUGINS: OnceLock<Mutex<WasmMap>> = OnceLock::new();
static DISABLED: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();

fn disabled() -> &'static Mutex<std::collections::HashSet<String>> {
    DISABLED.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

fn plugins() -> &'static Mutex<WasmMap> {
    PLUGINS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn root() -> std::path::PathBuf {
    std::env::var("M7S_WASM_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("data/wasm"))
}

/// Load a `.wasm`/`.wat` file relative to the WASM directory.
pub fn load_file(name: &str, relative: &str) -> Result<(), String> {
    let pubkey = std::env::var("M7S_WASM_PUBKEY").ok().and_then(|h| decode_hex_32(&h));
    load_file_with_key(name, relative, pubkey.as_ref())
}

fn load_file_with_key(name: &str, relative: &str, pubkey: Option<&[u8; 32]>) -> Result<(), String> {
    let root = root();
    if !allowed_by(&std::env::var("M7S_WASM_ALLOW").unwrap_or_default(), name) {
        return Err("wasm plugin not allowed".to_string());
    }
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let path = root.join(relative);
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(&root) {
        return Err("path escapes WASM directory".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    if let Some(pubkey) = pubkey {
        let sig_path = std::path::PathBuf::from(format!("{}.sig", path.display()));
        let sig = std::fs::read(&sig_path).map_err(|e| format!("WASM signature missing: {e}"))?;
        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, pubkey)
            .verify(&bytes, &sig)
            .map_err(|_| "WASM signature invalid".to_string())?;
    }
    WasmPlugin::load(&bytes)?;
    plugins()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(name.to_string(), (bytes, path.clone()));
    Ok(())
}

pub fn names() -> Vec<String> {
    let mut v: Vec<String> = plugins()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .cloned()
        .collect();
    v.sort();
    v
}

pub fn unload(name: &str) -> bool {
    plugins().lock().unwrap_or_else(|e| e.into_inner()).remove(name).is_some()
}
pub fn reload(name: &str) -> Result<(), String> {
    let path = plugins()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(name)
        .map(|(_, p)| p.clone())
        .ok_or_else(|| "wasm plugin not found".to_string())?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    WasmPlugin::load(&bytes)?;
    plugins()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(name.to_string(), (bytes, path));
    Ok(())
}

pub fn set_enabled(name: &str, enabled: bool) -> bool {
    if !plugins().lock().unwrap_or_else(|e| e.into_inner()).contains_key(name) {
        return false;
    }
    let mut d = disabled().lock().unwrap_or_else(|e| e.into_inner());
    if enabled { d.remove(name); } else { d.insert(name.to_string()); }
    true
}

pub fn is_enabled(name: &str) -> bool {
    !disabled().lock().unwrap_or_else(|e| e.into_inner()).contains(name)
}

pub fn status() -> Vec<(String, bool)> {
    let d = disabled().lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<(String, bool)> = plugins()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .map(|k| (k.clone(), !d.contains(k)))
        .collect();
    out.sort();
    out
}


pub fn process_registered(name: &str, input: &[u8]) -> Result<Vec<u8>, String> {
    if !is_enabled(name) { return Err("wasm plugin disabled".to_string()); }
    let bytes = plugins()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(name)
        .map(|(b, _)| b.clone())
        .ok_or_else(|| "wasm plugin not found".to_string())?;
    let mut p = WasmPlugin::load(&bytes)?;
    p.process(input)
}

pub fn process_frame_registered(
    name: &str,
    tag_type: u8,
    timestamp: u32,
    input: &[u8],
) -> Result<Vec<u8>, String> {
    let bytes = plugins()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(name)
        .map(|(b, _)| b.clone())
        .ok_or_else(|| "wasm plugin not found".to_string())?;
    let mut p = WasmPlugin::load(&bytes)?;
    match p.process_frame(tag_type, timestamp, input)? {
        Some(out) => Ok(out),
        None => p.process(input),
    }
}

pub fn process_av_registered(name: &str, frame: &AvFrame) -> Result<Option<AvFrame>, String> {
    if !is_enabled(name) { return Err("wasm plugin disabled".to_string()); }
    let bytes = plugins()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(name)
        .map(|(b, _)| b.clone())
        .ok_or_else(|| "wasm plugin not found".to_string())?;
    let mut p = WasmPlugin::load(&bytes)?;
    p.process_av_frame(frame)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 { return None; }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

#[cfg(test)]
#[cfg(test)]
mod registry_tests {
    use super::*;

    const WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 1024))
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    global.get $next local.set $p
    global.get $next local.get $n i32.add global.set $next
    local.get $p)
  (func (export "transform") (param $p i32) (param $n i32) (result i64)
    local.get $p i64.extend_i32_u i64.const 32 i64.shl
    local.get $n i64.extend_i32_u i64.or)
  (func (export "transform_frame") (param $t i32) (param $ts i32) (param $p i32) (param $n i32) (result i64)
    local.get $p i64.extend_i32_u i64.const 32 i64.shl
    local.get $n i64.extend_i32_u i64.or)
  (func (export "transform_av") (param $p i32) (param $n i32) (result i64)
    local.get $p i64.extend_i32_u i64.const 32 i64.shl
    local.get $n i64.extend_i32_u i64.or))
"#;

    #[test]
    fn loads_and_processes() {
        let dir = std::path::PathBuf::from(format!(
            "{}/../../target/wasm-test-{}",
            env!("CARGO_MANIFEST_DIR"),
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("id.wat"), WAT).unwrap();
        unsafe { std::env::set_var("M7S_WASM_DIR", &dir) };
        load_file("id", "id.wat").unwrap();
        assert_eq!(process_registered("id", b"xy").unwrap(), b"xy");
        assert!(names().contains(&"id".to_string()));
        let assets = market();
        assert!(assets.iter().any(|a| a.0 == "id" && a.3));
        let changed = WAT.replace("i32.const 1024", "i32.const 2048");
        std::fs::write(dir.join("id.wat"), changed).unwrap();
        assert!(hot_reload().contains(&"id".to_string()));
        assert_eq!(process_registered("id", b"xy").unwrap(), b"xy");
        assert!(load_file("bad", "../escape.wat").is_err());
        std::fs::write(dir.join("id.wat"), b"bad").unwrap();
        assert!(reload("id").is_err());
        std::fs::write(dir.join("id.wat"), WAT).unwrap();
        reload("id").unwrap();
        assert_eq!(process_registered("id", b"xy").unwrap(), b"xy");
        assert!(set_enabled("id", false));
        assert!(process_registered("id", b"xy").is_err());
        assert!(set_enabled("id", true));
        assert_eq!(process_registered("id", b"xy").unwrap(), b"xy");
        use ring::signature::KeyPair;
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let keypair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let sig = keypair.sign(WAT.as_bytes());
        std::fs::write(dir.join("id.wat.sig"), sig.as_ref()).unwrap();
        let pubkey: [u8; 32] = keypair.public_key().as_ref().try_into().unwrap();
        assert!(load_file_with_key("signed", "id.wat", Some(&pubkey)).is_ok());
        std::fs::write(dir.join("id.wat"), b"bad").unwrap();
        assert!(load_file_with_key("signed", "id.wat", Some(&pubkey)).is_err());
        std::fs::write(dir.join("id.wat"), WAT).unwrap();
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(unhex(&hex(b"abc")).unwrap(), b"abc");
    }
}

use crate::flv::FlvTag;
use crate::stream::Stream;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

fn to_av(tag: &crate::flv::FlvTag) -> AvFrame {
    AvFrame {
        tag_type: tag.tag_type,
        keyframe: tag.tag_type == 9 && !tag.data.is_empty() && (tag.data[0] >> 4) == 1,
        timestamp: tag.timestamp,
        data: tag.data.clone(),
    }
}

#[derive(Default)]
pub struct ChainStats {
    pub processed: std::sync::atomic::AtomicU64,
    pub errors: std::sync::atomic::AtomicU64,
    pub stopped: AtomicBool,
    pub backpressure: std::sync::atomic::AtomicU64,
}

type ChainEntry = (String, String, String, std::sync::Arc<ChainStats>);
type ChainMap = HashMap<String, ChainEntry>;

static CHAINS: OnceLock<Mutex<ChainMap>> = OnceLock::new();

fn chains() -> &'static Mutex<ChainMap> {
    CHAINS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn chain_status() -> Vec<serde_json::Value> {
    chains()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .map(|(name, source, sink, s)| {
            serde_json::json!({
                "name": name,
                "source": source,
                "sink": sink,
                "processed": s.processed.load(std::sync::atomic::Ordering::Relaxed),
                "errors": s.errors.load(std::sync::atomic::Ordering::Relaxed),
                "stopped": s.stopped.load(std::sync::atomic::Ordering::Relaxed),
                "backpressure": s.backpressure.load(std::sync::atomic::Ordering::Relaxed)
            })
        })
        .collect()
}

/// Attach a loaded WASM plugin between two streams until `stop` is set.
pub async fn run_chain(
    mut plugin: WasmPlugin,
    source: std::sync::Arc<Stream>,
    sink: std::sync::Arc<Stream>,
    stop: std::sync::Arc<AtomicBool>,
    stats: std::sync::Arc<ChainStats>,
) {
    let mut reader = source.dispatch_reader();
    let mut consecutive = 0u32;
    let mut last_dropped = 0u64;
    while !stop.load(AtomicOrdering::Relaxed) {
        let mut got = false;
        while let Some(tag) = reader.try_next() {
            got = true;
            let frame = to_av(&tag);
            match plugin.process_av_frame(&frame) {
                Ok(Some(out)) => {
                    sink.push(std::sync::Arc::new(FlvTag::new(
                        out.tag_type,
                        out.timestamp,
                        out.data,
                    )));
                    stats.processed.fetch_add(1, AtomicOrdering::Relaxed);
                    consecutive = 0;
                }
                Ok(None) => {
                    sink.push(tag);
                    stats.processed.fetch_add(1, AtomicOrdering::Relaxed);
                    consecutive = 0;
                }
                Err(e) => {
                    consecutive += 1;
                    stats.errors.fetch_add(1, AtomicOrdering::Relaxed);
                    log::warn!("wasm chain transform: {e}");
                    if consecutive >= 10 {
                        stats.stopped.store(true, AtomicOrdering::Relaxed);
                        break;
                    }
                }
            }
        }
        let dropped = reader.dropped();
        if dropped > last_dropped {
            last_dropped = dropped;
            stats.backpressure.fetch_add(1, AtomicOrdering::Relaxed);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        if stats.stopped.load(AtomicOrdering::Relaxed) {
            break;
        }
        if !got {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }
    stats.stopped.store(true, AtomicOrdering::Relaxed);
}

/// Load a registered WASM plugin by name and attach it to source -> sink.
pub fn attach_registered(
    name: &str,
    source: std::sync::Arc<Stream>,
    sink: std::sync::Arc<Stream>,
) -> Result<std::sync::Arc<AtomicBool>, String> {
    if !is_enabled(name) {
        return Err("wasm plugin disabled".to_string());
    }
    let bytes = plugins()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(name)
        .map(|(b, _)| b.clone())
        .ok_or_else(|| "wasm plugin not found".to_string())?;
    let plugin = WasmPlugin::load(&bytes)?;
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let stats = std::sync::Arc::new(ChainStats::default());
    let key = format!("{name}:{}->{}", source.key(), sink.key());
    chains().lock().unwrap_or_else(|e| e.into_inner()).insert(
        key,
        (name.to_string(), source.key(), sink.key(), stats.clone()),
    );
    tokio::spawn(run_chain(plugin, source, sink, flag, stats));
    Ok(stop)
}

#[cfg(test)]
mod chain_tests {
    use super::*;

    const WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 1024))
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    global.get $next local.set $p
    global.get $next local.get $n i32.add global.set $next
    local.get $p)
  (func (export "transform") (param $p i32) (param $n i32) (result i64)
    local.get $p i64.extend_i32_u i64.const 32 i64.shl
    local.get $n i64.extend_i32_u i64.or)
  (func (export "transform_av") (param $p i32) (param $n i32) (result i64)
    local.get $p i64.extend_i32_u i64.const 32 i64.shl
    local.get $n i64.extend_i32_u i64.or))
"#;

    #[test]
    fn chains_frame_to_sink() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let mgr = crate::stream::StreamManager::new(10);
                let source = mgr.get_or_create("live", "src");
                let sink = mgr.get_or_create("live", "dst");
                let plugin = WasmPlugin::load(WAT.as_bytes()).unwrap();
                let stop = std::sync::Arc::new(AtomicBool::new(false));
                let stats = std::sync::Arc::new(ChainStats::default());
                tokio::spawn(run_chain(plugin, source.clone(), sink.clone(), stop.clone(), stats.clone()));
                source.push(std::sync::Arc::new(FlvTag::new(9, 11, vec![0x17, 1, 2, 3])));
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                stop.store(true, AtomicOrdering::Relaxed);
                let tags = sink.cache_snapshot();
                assert_eq!(tags.len(), 1);
                assert_eq!(tags[0].timestamp, 11);
                assert!(stats.processed.load(AtomicOrdering::Relaxed) >= 1);
                source.leave();
                sink.leave();
            });
    }
}

/// List `.wasm`/`.wat` files in the WASM directory (market view).
pub fn market() -> Vec<(String, String, u64, bool, bool)> {
    let root = root();
    let installed = names();
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if ext != "wasm" && ext != "wat" {
            continue;
        }
        let Some(file) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let name = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or(file)
            .to_string();
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let is_installed = installed.iter().any(|n| n == &name);
        out.push((name.clone(), file.to_string(), size, is_installed, is_enabled(&name)));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Reload installed plugins whose source file bytes changed.
pub fn hot_reload() -> Vec<String> {
    let entries: Vec<(String, std::path::PathBuf, Vec<u8>)> = plugins()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|(name, (bytes, path))| (name.clone(), path.clone(), bytes.clone()))
        .collect();
    let mut reloaded = Vec::new();
    for (name, path, old) in entries {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if bytes == old {
            continue;
        }
        if WasmPlugin::load(&bytes).is_err() {
            continue;
        }
        plugins()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(name.clone(), (bytes, path));
        reloaded.push(name);
    }
    reloaded
}

/// Poll the WASM directory and hot-reload changed plugins.
pub fn start_watcher(interval_secs: u64) {
    let interval = interval_secs.max(1);
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(interval));
        let changed = hot_reload();
        if !changed.is_empty() {
            log::info!("WASM hot reload: {:?}", changed);
        }
    });
}

fn decode_hex_32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn allowed_by(list: &str, name: &str) -> bool {
    let list = list.trim();
    if list.is_empty() {
        return true;
    }
    list.split(',').map(str::trim).any(|n| n.eq_ignore_ascii_case(name))
}

pub fn permissions() -> serde_json::Value {
    serde_json::json!({
        "allow": std::env::var("M7S_WASM_ALLOW").unwrap_or_default(),
        "fuel": std::env::var("M7S_WASM_FUEL").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(50_000_000),
        "require_signature": std::env::var("M7S_WASM_PUBKEY").is_ok(),
    })
}
