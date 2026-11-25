use std::sync::Arc;
use std::time::Duration;

use m7s_rs::config::{AuthConfig, ClusterConfig, Config, RecordConfig, ServerConfig};
use m7s_rs::record::RecordManager;
use m7s_rs::rtmp;
use m7s_rs::rtmp::amf0::{self, Amf0};
use m7s_rs::rtmp::chunk::{self, ChunkWriter, RtmpMessage};
use m7s_rs::stream::StreamManager;
use m7s_rs::tls;
use rustls::RootCertStore;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn rtmps_publish_reaches_stream_manager() {
    runtime().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let streams = StreamManager::new(10);
        let (acceptor, cert, _key) = tls::self_signed().unwrap();
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
        tokio::spawn(rtmp::run_tls(
            listener,
            streams.clone(),
            cfg,
            recorder,
            acceptor,
        ));

        let mut roots = RootCertStore::empty();
        roots.add(cert).unwrap();
        let client = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(client));
        let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut tls = connector
            .connect("localhost".try_into().unwrap(), tcp)
            .await
            .unwrap();
        chunk::client_handshake(&mut tls).await.unwrap();
        let mut out = ChunkWriter::new();
        out.set_chunk_size(4096);
        let connect = Amf0::Object(vec![
            ("app".to_string(), Amf0::String("live".to_string())),
            (
                "tcUrl".to_string(),
                Amf0::String("rtmps://localhost/live".to_string()),
            ),
        ]);
        send(&mut out, &mut tls, "connect", 1.0, vec![connect]).await;
        send(&mut out, &mut tls, "createStream", 2.0, vec![Amf0::Null]).await;
        send(
            &mut out,
            &mut tls,
            "publish",
            3.0,
            vec![Amf0::Null, Amf0::String("rtmps".to_string())],
        )
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        out.write_message(
            &mut tls,
            &RtmpMessage {
                type_id: 9,
                stream_id: 1,
                timestamp: 0,
                payload: vec![0x17, 1, 2, 3],
            },
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(streams.get("live", "rtmps").is_some());
    });
}

async fn send<W: tokio::io::AsyncWrite + Unpin>(
    out: &mut ChunkWriter,
    writer: &mut W,
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
        writer,
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
