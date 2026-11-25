use std::time::Duration;

use m7s_rs::cluster::Cluster;
use m7s_rs::config::{AuthConfig, ClusterConfig, Config, ServerConfig};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::record::RecordManager;
use m7s_rs::rtmp;
use m7s_rs::rtmp::amf0::{self, Amf0};
use m7s_rs::rtmp::chunk::{self, ChunkWriter, RtmpMessage};
use m7s_rs::stream::StreamManager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn rtmp_publish_appears_on_http_flv() {
    runtime().block_on(async {
        let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_port = http_listener.local_addr().unwrap().port();
        let rtmp_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let rtmp_port = rtmp_listener.local_addr().unwrap().port();
        let streams = StreamManager::new(10);
        let cluster = Cluster::new(
            ClusterConfig {
                enabled: false,
                advertise: format!("127.0.0.1:{http_port}"),
                ..ClusterConfig::default()
            },
            streams.clone(),
        );
        let state = AppState {
            streams: streams.clone(),
            cluster,
            hls: HlsManager::new(streams.clone(), m7s_rs::config::HlsConfig::default()),
            node_id: "test".to_string(),
            api_token: String::new(),
        };
        tokio::spawn(http_run(http_listener, state));
        let cfg = Config {
            server: ServerConfig {
                http_bind: format!("127.0.0.1:{http_port}"),
                rtmp_bind: format!("127.0.0.1:{rtmp_port}"),
                gop_cache: 10,
                backlog: 1024,
            },
            cluster: ClusterConfig::default(),
            auth: AuthConfig::default(),
            ..Config::default()
        };
        let recorder = RecordManager::new(streams.clone(), m7s_rs::config::RecordConfig::default());
        tokio::spawn(rtmp::run(rtmp_listener, streams.clone(), cfg, recorder));

        let mut tcp = TcpStream::connect(("127.0.0.1", rtmp_port)).await.unwrap();
        chunk::client_handshake(&mut tcp).await.unwrap();
        let mut out = ChunkWriter::new();
        out.set_chunk_size(4096);
        let connect = Amf0::Object(vec![
            ("app".to_string(), Amf0::String("live".to_string())),
            (
                "tcUrl".to_string(),
                Amf0::String("rtmp://localhost/live".to_string()),
            ),
        ]);
        send_command(&mut out, &mut tcp, "connect", 1.0, vec![connect]).await;
        send_command(&mut out, &mut tcp, "createStream", 2.0, vec![Amf0::Null]).await;
        send_command(
            &mut out,
            &mut tcp,
            "publish",
            3.0,
            vec![Amf0::Null, Amf0::String("test".to_string())],
        )
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        out.write_message(
            &mut tcp,
            &RtmpMessage {
                type_id: 9,
                stream_id: 1,
                timestamp: 0,
                payload: vec![0x17, 1, 2, 3],
            },
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(streams.get("live", "test").is_some());

        let mut client = TcpStream::connect(("127.0.0.1", http_port)).await.unwrap();
        client
            .write_all(b"GET /flv/live/test.flv HTTP/1.1\r\nHost: test\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 256];
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while buf.len() < 13 + 19 && tokio::time::Instant::now() < deadline {
            let n = tokio::time::timeout(Duration::from_millis(300), client.read(&mut chunk))
                .await
                .unwrap()
                .unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let pos = buf
            .windows(3)
            .position(|w| w == b"FLV")
            .expect("FLV header");
        assert!(buf.len() >= pos + 13 + 11, "FLV tag missing");
        assert_eq!(&buf[pos + 13..pos + 16], &[9, 0, 0]);
    });
}

async fn send_command(
    out: &mut ChunkWriter,
    tcp: &mut TcpStream,
    name: &str,
    tx: f64,
    args: Vec<Amf0>,
) {
    let mut values = vec![Amf0::String(name.to_string()), Amf0::Number(tx)];
    values.extend(args);
    let mut payload = Vec::new();
    for v in values {
        payload.extend_from_slice(&amf0::encode(&v));
    }
    out.write_message(
        tcp,
        &RtmpMessage {
            type_id: 20,
            stream_id: 0,
            timestamp: 0,
            payload,
        },
    )
    .await
    .unwrap();
}
