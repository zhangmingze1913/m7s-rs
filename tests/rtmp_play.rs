use std::time::Duration;

use m7s_rs::config::{AuthConfig, ClusterConfig, Config, RecordConfig, ServerConfig};
use m7s_rs::record::RecordManager;
use m7s_rs::rtmp;
use m7s_rs::rtmp::amf0::{self, Amf0};
use m7s_rs::rtmp::chunk::{self, ChunkReader, ChunkWriter, RtmpMessage};
use m7s_rs::stream::StreamManager;
use tokio::net::{TcpListener, TcpStream};

#[test]
fn rtmp_play_returns_cached_tag() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            let cfg = Config {
                server: ServerConfig {
                    http_bind: "127.0.0.1:0".to_string(),
                    rtmp_bind: format!("127.0.0.1:{port}"),
                    gop_cache: 10,
                    backlog: 128,
                },
                cluster: ClusterConfig::default(),
                auth: AuthConfig::default(),
                ..Config::default()
            };
            let recorder = RecordManager::new(streams.clone(), RecordConfig::default());
            tokio::spawn(rtmp::run(listener, streams.clone(), cfg, recorder));

            let mut pub_tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            chunk::client_handshake(&mut pub_tcp).await.unwrap();
            let mut pub_out = ChunkWriter::new();
            pub_out.set_chunk_size(4096);
            send(&mut pub_out, &mut pub_tcp, "connect", 1.0, vec![Amf0::Object(vec![
                ("app".into(), Amf0::String("live".into())),
                ("tcUrl".into(), Amf0::String("rtmp://localhost/live".into())),
            ])]).await;
            send(&mut pub_out, &mut pub_tcp, "createStream", 2.0, vec![Amf0::Null]).await;
            send(&mut pub_out, &mut pub_tcp, "publish", 3.0, vec![Amf0::Null, Amf0::String("play".into())]).await;
            tokio::time::sleep(Duration::from_millis(200)).await;
            pub_out.write_message(&mut pub_tcp, &RtmpMessage {
                type_id: 9,
                stream_id: 1,
                timestamp: 0,
                payload: vec![0x17, 1, 2, 3],
            }).await.unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;

            let cached = streams.get("live", "play").unwrap().cache_snapshot();
            assert!(cached.iter().any(|t| t.tag_type == 9 && t.data == [0x17, 1, 2, 3]), "publisher cache: {cached:?}");
            let mut play_tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            chunk::client_handshake(&mut play_tcp).await.unwrap();
            let mut play_out = ChunkWriter::new();
            play_out.set_chunk_size(4096);
            send(&mut play_out, &mut play_tcp, "connect", 1.0, vec![Amf0::Object(vec![
                ("app".into(), Amf0::String("live".into())),
                ("tcUrl".into(), Amf0::String("rtmp://localhost/live".into())),
            ])]).await;
            send(&mut play_out, &mut play_tcp, "createStream", 2.0, vec![Amf0::Null]).await;
            send(&mut play_out, &mut play_tcp, "play", 3.0, vec![Amf0::Null, Amf0::String("play".into())]).await;

            let mut reader = ChunkReader::new();
            reader.set_chunk_size(4096);
            let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
            let mut got = false;
            while tokio::time::Instant::now() < deadline {
                let msg = tokio::time::timeout(Duration::from_millis(500), reader.read_message(&mut play_tcp)).await;
                if let Ok(Ok(msg)) = msg {
                    if msg.type_id == 9 && msg.payload == [0x17, 1, 2, 3] {
                        got = true;
                        break;
                    }
                }
            }
            assert!(got, "no RTMP play tag");
            assert!(streams.get("live", "play").is_some());
        });
}

async fn send(out: &mut ChunkWriter, tcp: &mut TcpStream, name: &str, tx: f64, args: Vec<Amf0>) {
    let mut values = vec![Amf0::String(name.to_string()), Amf0::Number(tx)];
    values.extend(args);
    let mut payload = Vec::new();
    for v in values {
        payload.extend_from_slice(&amf0::encode(&v));
    }
    out.write_message(tcp, &RtmpMessage {
        type_id: 20,
        stream_id: 0,
        timestamp: 0,
        payload,
    }).await.unwrap();
}
