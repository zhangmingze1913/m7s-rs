use std::sync::Arc;

use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::flv::FlvTag;
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn sei_http_inserts_user_data() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(5);
            let stream = streams.get_or_create("live", "sei");
            stream.push(Arc::new(FlvTag::new(9, 0, vec![0x17, 0, 0, 0, 0, 1, 0x42, 1, 2, 3])));
            let state = AppState {
                streams: streams.clone(),
                cluster: Cluster::new(ClusterConfig { enabled: false, ..ClusterConfig::default() }, StreamManager::new(5)),
                hls: HlsManager::new(streams.clone(), HlsConfig::default()),
                node_id: "sei".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(b"GET /api/sei?stream=live/sei&text=hello HTTP/1.1\r\nHost: t\r\n\r\n")
                .await
                .unwrap();
            let mut buf = Vec::new();
            s.read_to_end(&mut buf).await.unwrap();
            let text = String::from_utf8_lossy(&buf);
            assert!(text.contains("\"code\":0"), "{text}");

            let tags = stream.cache_snapshot();
            let injected = tags
                .iter()
                .rev()
                .find(|t| t.tag_type == 9 && t.data.len() > 5)
                .expect("sei tag");
            assert!(injected.data.windows(2).any(|w| w == [6, 5]));
            assert!(injected.data.windows(5).any(|w| w == b"hello"));
        });
}
