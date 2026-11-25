//! ONVIF plugin, re-exported from `m7s-plugin-onvif`.
pub use m7s_plugin_onvif::client::{
    Discovery, OnvifClient, discover, discover_at, parse_discovery, probe_message,
};
pub use m7s_plugin_onvif::{discovery, push_event, run, run_with_context};
