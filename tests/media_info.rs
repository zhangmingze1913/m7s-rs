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
fn media_info_reports_avc_and_asc() {
    let ff = std::env::var("M7S_FFMPEG").unwrap_or_else(|_| {
        let local = format!(
            "{}/node_modules/ffmpeg-static/ffmpeg.exe",
            env!("CARGO_MANIFEST_DIR")
        );
        if std::path::Path::new(&local).exists() {
            local
        } else {
            "ffmpeg".into()
        }
    });
    let output = std::process::Command::new(&ff)
        .args([
            "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
            "testsrc=size=320x240:rate=1", "-frames:v", "1", "-c:v", "libx264",
            "-x264-params", "keyint=1", "-f", "h264", "-",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let nals = m7s_codec::split_annexb(&output.stdout);
    let sps = nals.iter().find(|n| n[0] & 0x1f == 7).unwrap().to_vec();
    let pps = nals.iter().find(|n| n[0] & 0x1f == 8).unwrap().to_vec();
    let record = m7s_codec::build_avc_config(&sps, &pps);
    let asc = m7s_codec::build_asc(m7s_codec::AudioSpecificConfig {
        object_type: 2,
        sample_rate: 44100,
        channels: 2,
    })
    .unwrap();

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(5);
            let stream = streams.get_or_create("live", "info");
            let mut vcfg = vec![0x17, 0, 0, 0, 0];
            vcfg.extend_from_slice(&record);
            stream.push(Arc::new(FlvTag::new(9, 0, vcfg)));
            let mut acfg = vec![0xAF, 0];
            acfg.extend_from_slice(&asc);
            stream.push(Arc::new(FlvTag::new(8, 0, acfg)));
            let state = AppState {
                streams: streams.clone(),
                cluster: Cluster::new(ClusterConfig { enabled: false, ..ClusterConfig::default() }, StreamManager::new(5)),
                hls: HlsManager::new(streams, HlsConfig::default()),
                node_id: "info".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(b"GET /api/media/info?stream=live/info HTTP/1.1\r\nHost: t\r\n\r\n")
                .await
                .unwrap();
            let mut buf = Vec::new();
            s.read_to_end(&mut buf).await.unwrap();
            let text = String::from_utf8_lossy(&buf);
            assert!(text.contains("\"width\":320"), "{text}");
            assert!(text.contains("\"height\":240"), "{text}");
            assert!(text.contains("\"profile\":"), "{text}");
            assert!(text.contains("\"level\":"), "{text}");
            assert!(text.contains("\"sample_rate\":44100"), "{text}");
            assert!(text.contains("\"channels\":2"), "{text}");
        });
}
