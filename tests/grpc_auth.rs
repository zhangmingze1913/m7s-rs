
use bytes::Bytes;
use h2::client;
use http::Request;
use m7s_rs::grpc::run as grpc_run;
use m7s_rs::stream::StreamManager;
use tokio::net::{TcpListener, TcpStream};

#[test]
fn grpc_token_auth() {
    unsafe { std::env::set_var("M7S_GRPC_TOKEN", "secret") };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let streams = StreamManager::new(10);
            streams.get_or_create("live", "grpc-auth");
            tokio::spawn(grpc_run(listener, streams));

            let socket = TcpStream::connect(addr).await.unwrap();
            let (mut send_request, conn) = client::handshake(socket).await.unwrap();
            tokio::spawn(async move { let _ = conn.await; });

            let (status, body) = call(&mut send_request, None).await;
            assert_eq!(status, "16");
            assert!(body.is_empty());

            let (status, body) = call(&mut send_request, Some("Bearer secret")).await;
            assert_eq!(status, "0");
            assert!(body.len() > 5);
        });
}

async fn call(
    send_request: &mut h2::client::SendRequest<Bytes>,
    auth: Option<&str>,
) -> (String, Vec<u8>) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/m7s.StreamService/ListStreams")
        .header("content-type", "application/grpc")
        .header("te", "trailers");
    if let Some(auth) = auth {
        builder = builder.header("authorization", auth);
    }
    let req = builder.body(()).unwrap();
    let (response, mut body) = send_request.send_request(req, false).unwrap();
    body.send_data(Bytes::from_static(&[0, 0, 0, 0, 0]), true).unwrap();
    let response = response.await.unwrap();
    let mut resp_body = response.into_body();
    let mut data = Vec::new();
    while let Some(chunk) = resp_body.data().await {
        data.extend_from_slice(&chunk.unwrap());
    }
    let trailers = resp_body.trailers().await.unwrap();
    let status = trailers
        .and_then(|t| t.get("grpc-status").and_then(|v| v.to_str().ok().map(str::to_string)))
        .unwrap_or_default();
    (status, data)
}
