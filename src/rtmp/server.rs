//! RTMP server adapter: implementation lives in `m7s-plugin-rtmp`.

use std::sync::Arc;

use anyhow::Result;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use crate::config::Config;
use crate::record::RecordManager;
use crate::stream::StreamManager;

struct RecorderHook(Arc<RecordManager>);

impl m7s_plugin_rtmp::server::RecordHook for RecorderHook {
    fn ensure(&self, app: &str, name: &str) {
        self.0.ensure(app, name);
    }
}

fn options(cfg: &Config) -> m7s_plugin_rtmp::server::RtmpConfig {
    m7s_plugin_rtmp::server::RtmpConfig {
        publish_key: cfg.auth.publish_key.clone(),
    }
}

pub async fn run(
    listener: TcpListener,
    streams: Arc<StreamManager>,
    cfg: Config,
    recorder: Arc<RecordManager>,
) -> Result<()> {
    let hook: Arc<dyn m7s_plugin_rtmp::server::RecordHook> = Arc::new(RecorderHook(recorder));
    m7s_plugin_rtmp::server::run(
        listener,
        streams as Arc<dyn m7s_sdk::StreamApi>,
        options(&cfg),
        Some(hook),
    )
    .await
}

pub async fn run_tls(
    listener: TcpListener,
    streams: Arc<StreamManager>,
    cfg: Config,
    recorder: Arc<RecordManager>,
    acceptor: TlsAcceptor,
) -> Result<()> {
    let hook: Arc<dyn m7s_plugin_rtmp::server::RecordHook> = Arc::new(RecorderHook(recorder));
    m7s_plugin_rtmp::server::run_tls(
        listener,
        streams as Arc<dyn m7s_sdk::StreamApi>,
        options(&cfg),
        Some(hook),
        acceptor,
    )
    .await
}
