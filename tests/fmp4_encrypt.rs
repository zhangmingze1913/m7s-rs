use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::flv::FlvTag;
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const KEY_HEX: &str = "00112233445566778899aabbccddeeff";

#[test]
fn fmp4_aes128() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let streams = StreamManager::new(10);
            let stream = streams.get_or_create("live", "fme");
            let cfg = vec![0x17, 0, 0, 0, 0, 1, 0x42, 0, 0x1e, 0xff, 0xe1, 0, 4, 0x67, 0x42, 0, 0x1e, 1, 0, 4, 0x68, 1, 2, 3];
            stream.push(Arc::new(FlvTag::new(9, 0, cfg)));
            stream.push(Arc::new(FlvTag::new(9, 0, vec![0x17, 1, 0, 0, 0, 0, 0, 0, 0x65, 1, 2])));
            stream.push(Arc::new(FlvTag::new(8, 0, vec![0xAF, 0x00, 0x12, 0x10])));
            stream.push(Arc::new(FlvTag::new(8, 0, vec![0xAF, 0x01, 1, 2, 3, 4])));
            let hls_cfg = HlsConfig { encrypt: true, key: KEY_HEX.to_string(), ..HlsConfig::default() };
            let state = AppState {
                streams: streams.clone(),
                cluster: Cluster::new(ClusterConfig { enabled: false, ..ClusterConfig::default() }, StreamManager::new(10)),
                hls: HlsManager::new(streams, hls_cfg),
                node_id: "fme".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(listener, state));

            let (status, body) = get(port, "/fmp4/live/fme/av.m3u8").await;
            assert_eq!(status, 200);
            let text = String::from_utf8_lossy(&body);
            assert!(text.contains("#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\""), "{text}");

            let (status, key) = get(port, "/fmp4/live/fme/key.bin").await;
            assert_eq!(status, 200);
            assert_eq!(key.len(), 16);

            let (status, enc) = get(port, "/fmp4/live/fme/av-0.m4s").await;
            assert_eq!(status, 200);
            assert_eq!(enc.len() % 16, 0);
            assert!(!enc.windows(4).any(|w| w == b"moof"), "segment should be encrypted");

            use aes::cipher::block_padding::Pkcs7;
            use aes::cipher::{BlockModeDecrypt, KeyIvInit};
            let mut key = [0u8; 16];
            for (i, slot) in key.iter_mut().enumerate() {
                *slot = u8::from_str_radix(&KEY_HEX[i * 2..i * 2 + 2], 16).unwrap();
            }
            let dec = cbc::Decryptor::<aes::Aes128>::new((&key).into(), (&[0u8; 16]).into())
                .decrypt_padded_vec::<Pkcs7>(&enc)
                .unwrap();
            assert!(dec.windows(4).any(|w| w == b"moof"), "decrypted moof");

            let (status, ll) = get(port, "/fmp4/live/fme/index.m3u8?_HLS_msn=0").await;
            assert_eq!(status, 200);
            let ll = String::from_utf8_lossy(&ll);
            assert!(ll.contains("#EXT-X-KEY"), "{ll}");
            let (status, part) = get(port, "/fmp4/live/fme/part-0-0.m4s").await;
            assert_eq!(status, 200);
            assert!(!part.windows(4).any(|w| w == b"moof"));
            let dec = cbc::Decryptor::<aes::Aes128>::new((&key).into(), (&[0u8; 16]).into())
                .decrypt_padded_vec::<Pkcs7>(&part)
                .unwrap();
            assert!(dec.windows(4).any(|w| w == b"moof"), "decrypted part moof");
        });
}

async fn get(port: u16, path: &str) -> (u16, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: t\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    let status = text.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let body = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| buf[p + 4..].to_vec())
        .unwrap_or_default();
    (status, body)
}
