use std::sync::Arc;

use m7s_rs::flv::FlvTag;
use m7s_rs::onvif::run as onvif_run;
use m7s_rs::onvif_pull::pull;
use m7s_rs::rtsp::run as rtsp_run;
use m7s_rs::stream::StreamManager;
use tokio::net::TcpListener;

#[test]
fn onvif_stream_uri_to_rtsp_pull() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let rtsp_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let rtsp_port = rtsp_listener.local_addr().unwrap().port();
            unsafe {
                std::env::set_var(
                    "M7S_ONVIF_STREAM_URI",
                    format!("rtsp://127.0.0.1:{rtsp_port}/live/src"),
                )
            };
            let streams = StreamManager::new(10);
            let src = streams.get_or_create("live", "src");
            src.push(Arc::new(FlvTag::new(
                9,
                0,
                vec![0x17, 0, 0, 0, 0, 1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4, 0x68, 1, 2, 3],
            )));
            src.push(Arc::new(FlvTag::new(9, 40, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 0x65, 1, 2])));
            tokio::spawn(rtsp_run(rtsp_listener, streams.clone()));

            let onvif_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let onvif_port = onvif_listener.local_addr().unwrap().port();
            tokio::spawn(onvif_run(onvif_listener));
            let endpoint = format!("http://127.0.0.1:{onvif_port}/onvif/device_service");

            let pull_streams = streams.clone();
            tokio::spawn(async move {
                if let Err(e) = pull(&endpoint, "profile1", "", "", pull_streams, "live", "pulled").await {
                    eprintln!("onvif pull: {e:#}");
                }
            });
            tokio::time::sleep(std::time::Duration::from_millis(900)).await;
            let dest = streams.get("live", "pulled").expect("pulled stream");
            let tags = dest.cache_snapshot();
            assert!(!tags.is_empty(), "no tags");
            assert!(tags.iter().any(|t| t.tag_type == 9 && t.data.first() == Some(&0x17)));
        });
}
