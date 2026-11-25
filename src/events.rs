//! Lightweight stream lifecycle event bus (SSE / plugins).

use std::sync::OnceLock;

use tokio::sync::broadcast;

static BUS: OnceLock<broadcast::Sender<String>> = OnceLock::new();

pub fn bus() -> &'static broadcast::Sender<String> {
    BUS.get_or_init(|| broadcast::channel(256).0)
}

pub fn publish(topic: &str, payload: &str) {
    let _ = bus().send(format!("{topic} {payload}"));
}

pub fn subscribe() -> broadcast::Receiver<String> {
    bus().subscribe()
}
