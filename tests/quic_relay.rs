use std::sync::Arc;

use m7s_rs::flv::FlvTag;
use m7s_rs::quic_cluster::run as quic_run;
use m7s_rs::stream::StreamManager;

#[test]
fn quic_relay_delivers_flv_tags() {
    let cfg_path = std::env::temp_dir().join(format!("m7s-quic-toggle-{}.json", std::process::id()));
    unsafe { std::env::set_var("M7S_PLUGIN_CONFIG", &cfg_path) };
    std::fs::write(&cfg_path, b"[]").unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let streams = StreamManager::new(10);
            let stream = streams.get_or_create("live", "quic");
            stream.push(Arc::new(FlvTag::new(9, 5, vec![0x17, 1, 2, 3])));
            tokio::spawn(quic_run(streams, "127.0.0.1:8180".to_string()));
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;

            let mut roots = rustls::RootCertStore::empty();
            let mut ca = std::io::BufReader::new(
                std::fs::File::open("tests/fixtures/ca.pem").unwrap(),
            );
            for cert in rustls_pemfile::certs(&mut ca) {
                roots.add(cert.unwrap()).unwrap();
            }
            let cfg = quinn::ClientConfig::with_root_certificates(Arc::new(roots)).unwrap();
            let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
            endpoint.set_default_client_config(cfg);
            let conn = endpoint
                .connect("127.0.0.1:9443".parse().unwrap(), "localhost")
                .unwrap()
                .await
                .unwrap();
            let (mut send, mut recv) = conn.open_bi().await.unwrap();
            send.write_all(b"live/quic\n").await.unwrap();

            let mut header = [0u8; 13];
            recv.read_exact(&mut header).await.unwrap();
            assert_eq!(&header[0..3], b"FLV");

            let mut tag_hdr = [0u8; 9];
            recv.read_exact(&mut tag_hdr).await.unwrap();
            assert_eq!(tag_hdr[0], 9);
            let len = u32::from_be_bytes([tag_hdr[5], tag_hdr[6], tag_hdr[7], tag_hdr[8]]) as usize;
            let mut data = vec![0u8; len];
            recv.read_exact(&mut data).await.unwrap();
            assert_eq!(data, vec![0x17, 1, 2, 3]);
        });
}
