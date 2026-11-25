use std::sync::Arc;

use m7s_rs::flv::FlvTag;
use m7s_rs::rtsp::run;
use m7s_rs::rtsp_pull::pull;
use m7s_rs::stream::StreamManager;
use tokio::net::TcpListener;

#[test]
fn rtsp_pull_video() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            let src = streams.get_or_create("live", "src");
            src.push(Arc::new(FlvTag::new(
                9,
                0,
                vec![0x17, 0, 0, 0, 0, 1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4, 0x68, 1, 2, 3],
            )));
            src.push(Arc::new(FlvTag::new(9, 40, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 0x65, 1, 2])));
            src.push(Arc::new(FlvTag::new(8, 0, vec![0xAF, 0x00, 0x12, 0x10])));
            src.push(Arc::new(FlvTag::new(8, 40, vec![0xAF, 0x01, 1, 2, 3, 4])));
            tokio::spawn(run(listener, streams.clone()));

            let url = format!("rtsp://127.0.0.1:{port}/live/src");
            let pull_streams = streams.clone();
            tokio::spawn(async move {
                let _ = pull(&url, pull_streams, "live", "pulled").await;
            });
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let live = streams.get("live", "src").unwrap();
            live.push(Arc::new(FlvTag::new(9, 80, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 0x65, 1, 2])));
            live.push(Arc::new(FlvTag::new(8, 80, vec![0xAF, 0x01, 1, 2, 3, 4])));
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
            let dest = streams.get("live", "pulled").expect("pulled stream");
            let tags = dest.cache_snapshot();
            assert!(!tags.is_empty(), "no pulled tags");
            assert!(tags.iter().any(|t| t.tag_type == 9 && t.data.first() == Some(&0x17)));
            assert!(tags.iter().any(|t| t.tag_type == 8 && t.data.get(1) == Some(&0x00)));
            assert!(tags.iter().any(|t| t.tag_type == 8 && t.data.get(1) == Some(&0x01)));
        });
}
