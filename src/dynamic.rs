//! Dynamic plugin loader (C ABI v1) for `.dll`/`.so` plugins.

use std::ffi::CStr;
use std::os::raw::{c_char, c_uchar};
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use libloading::Library;

type Transform = unsafe extern "C" fn(*const c_uchar, usize, *mut c_uchar, usize) -> isize;

pub struct DynamicPlugin {
    _lib: Library,
    transform: Transform,
    name: String,
}

unsafe impl Send for DynamicPlugin {}
unsafe impl Sync for DynamicPlugin {}

impl DynamicPlugin {
    pub fn load(path: &str) -> Result<Self> {
        unsafe {
            let lib = Library::new(path).with_context(|| format!("load {path}"))?;
            let abi: libloading::Symbol<unsafe extern "C" fn() -> u32> = lib
                .get(b"m7s_plugin_abi\0")
                .context("missing m7s_plugin_abi")?;
            if abi() != 1 {
                bail!("unsupported plugin ABI");
            }
            let name_fn: libloading::Symbol<unsafe extern "C" fn() -> *const c_char> = lib
                .get(b"m7s_plugin_name\0")
                .context("missing m7s_plugin_name")?;
            let name_ptr = name_fn();
            if name_ptr.is_null() {
                bail!("null plugin name");
            }
            let name = CStr::from_ptr(name_ptr).to_string_lossy().to_string();
            let transform = *lib
                .get::<Transform>(b"m7s_transform\0")
                .context("missing m7s_transform")?;
            Ok(Self {
                _lib: lib,
                transform,
                name,
            })
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn transform(&self, input: &[u8]) -> Option<Vec<u8>> {
        let mut out = vec![0u8; input.len().max(1)];
        let n = unsafe { (self.transform)(input.as_ptr(), input.len(), out.as_mut_ptr(), out.len()) };
        if n < 0 {
            return None;
        }
        out.truncate(n as usize);
        Some(out)
    }
}

static PLUGIN: OnceLock<Option<DynamicPlugin>> = OnceLock::new();

fn plugin() -> &'static Option<DynamicPlugin> {
    PLUGIN.get_or_init(|| {
        let path = std::env::var("M7S_PLUGIN_PATH").ok()?;
        match DynamicPlugin::load(&path) {
            Ok(p) => {
                log::info!("dynamic plugin loaded: {} from {path}", p.name());
                Some(p)
            }
            Err(e) => {
                log::warn!("dynamic plugin load failed: {e:#}");
                None
            }
        }
    })
}

pub fn info() -> serde_json::Value {
    match plugin() {
        Some(p) => serde_json::json!({"loaded": true, "name": p.name(), "abi": 1}),
        None => serde_json::json!({"loaded": false}),
    }
}

pub fn echo(msg: &str) -> Option<String> {
    let p = plugin().as_ref()?;
    let out = p.transform(msg.as_bytes())?;
    Some(String::from_utf8_lossy(&out).to_string())
}
