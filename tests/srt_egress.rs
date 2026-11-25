use std::sync::Arc;
use std::time::Duration;

use m7s_rs::flv::FlvTag;
use m7s_rs::srt::{push, run as srt_run};
use m7s_rs::stream::StreamManager;

#[test]
fn srt_egress_pushes_stream() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let probe = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            let port = probe.local_addr().unwrap().port();
            drop(probe);
            let dest = StreamManager::new(10);
            let dest_stream = dest.get_or_create("live", "srtout");
            let bind = format!("127.0.0.1:{port}");
            tokio::spawn(async move {
                let _ = srt_run(&bind, dest).await;
            });
            tokio::time::sleep(Duration::from_millis(200)).await;

            let source = StreamManager::new(10);
            let source_stream = source.get_or_create("live", "src");
            let cfg = vec![0x17, 0, 0, 0, 0, 1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4, 0x68, 1, 2, 3];
            source_stream.push(Arc::new(FlvTag::new(9, 0, cfg)));
            let target = format!("127.0.0.1:{port}");
            let src = source_stream.clone();
            tokio::spawn(async move {
                let _ = push(src, &target, "live/srtout").await;
            });
            tokio::time::sleep(Duration::from_millis(200)).await;

            for (ts, idr) in [(0u32, true), (500, false), (1000, true), (1500, false), (2000, true), (2500, false)] {
                let mut data = vec![0x17, 1, 0, 0, 0, 0, 0, 0, 4, if idr { 0x65 } else { 0x41 }, 1, 2, 3];
                if !idr {
                    data[8] = 4;
                }
                source_stream.push(Arc::new(FlvTag::new(9, ts, data)));
                tokio::time::sleep(Duration::from_millis(120)).await;
            }

            let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
            while tokio::time::Instant::now() < deadline {
                if dest_stream.cache_snapshot().iter().any(|t| t.tag_type == 9) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            panic!("SRT egress did not deliver video tag");
        });
}
