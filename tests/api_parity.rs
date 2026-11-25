use bytes::Bytes;
use h2::client;
use http::Request;
use m7s_rs::cluster::Cluster;
use m7s_rs::config::{ClusterConfig, HlsConfig};
use m7s_rs::grpc::{GetStreamRequest, StreamList, run as grpc_run};
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run};
use m7s_rs::stream::StreamManager;
use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn grpc_http_stream_parity() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let streams = StreamManager::new(10);
            let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let http_port = http_listener.local_addr().unwrap().port();
            let state = AppState {
                streams: streams.clone(),
                cluster: Cluster::new(ClusterConfig { enabled: false, ..ClusterConfig::default() }, StreamManager::new(10)),
                hls: HlsManager::new(streams.clone(), HlsConfig::default()),
                node_id: "parity".to_string(),
                api_token: String::new(),
            };
            tokio::spawn(http_run(http_listener, state));
            let grpc_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let grpc_addr = grpc_listener.local_addr().unwrap();
            tokio::spawn(grpc_run(grpc_listener, streams.clone()));

            let socket = TcpStream::connect(grpc_addr).await.unwrap();
            let (mut send_request, conn) = client::handshake(socket).await.unwrap();
            tokio::spawn(async move { let _ = conn.await; });

            let list = grpc_list(&mut send_request).await;
            assert!(list.streams.is_empty());
            let body = http_get(http_port, "/api/streams").await;
            assert!(String::from_utf8_lossy(&body).contains("\"streams\":[]"));

            let create = GetStreamRequest { key: "live/parity".to_string() };
            let mut payload = Vec::new();
            create.encode(&mut payload).unwrap();
            let reply = grpc_call(&mut send_request, "/m7s.StreamService/CreateStream", &payload).await;
            assert!(reply[0] != 0 || reply.len() > 5);

            let list = grpc_list(&mut send_request).await;
            assert!(list.streams.iter().any(|s| s.app == "live" && s.name == "parity"));
            let body = http_get(http_port, "/api/streams").await;
            assert!(String::from_utf8_lossy(&body).contains("live/parity"));

            let body = http_get(http_port, "/api/stream/delete?stream=live/parity").await;
            assert!(String::from_utf8_lossy(&body).contains("\"removed\":true"));

            let list = grpc_list(&mut send_request).await;
            assert!(list.streams.is_empty());
        });
}

async fn grpc_call(
    send: &mut client::SendRequest<Bytes>,
    path: &str,
    payload: &[u8],
) -> Vec<u8> {
    let mut framed = vec![0u8];
    framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    framed.extend_from_slice(payload);
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .body(())
        .unwrap();
    let (response, mut body) = send.send_request(req, false).unwrap();
    body.send_data(Bytes::from(framed), true).unwrap();
    let response = response.await.unwrap();
    let mut resp = response.into_body();
    let mut data = Vec::new();
    while let Some(chunk) = resp.data().await {
        data.extend_from_slice(&chunk.unwrap());
    }
    data
}

async fn grpc_list(send: &mut client::SendRequest<Bytes>) -> StreamList {
    let data = grpc_call(send, "/m7s.StreamService/ListStreams", &[]).await;
    StreamList::decode(&data[5..]).unwrap()
}

async fn http_get(port: u16, path: &str) -> Vec<u8> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: t\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| buf[p + 4..].to_vec())
        .unwrap_or_default()
}
