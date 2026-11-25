//! ONVIF → RTSP pull orchestration.

use std::sync::Arc;

use anyhow::Result;

use crate::onvif::OnvifClient;
use crate::stream::StreamManager;

pub async fn pull(
    endpoint: &str,
    profile: &str,
    user: &str,
    password: &str,
    streams: Arc<StreamManager>,
    app: &str,
    name: &str,
) -> Result<()> {
    let mut client = OnvifClient::new(endpoint);
    if !user.is_empty() {
        client = client.with_auth(user, password);
    }
    let uri = client.get_stream_uri(profile).await?;
    crate::rtsp_pull::pull(&uri, streams, app, name).await
}
