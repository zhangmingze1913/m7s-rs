use bytes::Bytes;
use http::Request;
use m7s_rs::grpc::{
    ChatReply, ChatRequest, GiftReply, GiftRequest, RoomReply, RoomRequest, RoomStatus,
    run as grpc_run,
};
use m7s_rs::stream::StreamManager;
use prost::Message;
use tokio::net::{TcpListener, TcpStream};

#[test]
fn grpc_room_matches_http_surface() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(grpc_run(listener, StreamManager::new(10)));
            let socket = TcpStream::connect(addr).await.unwrap();
            let (mut send, conn) = h2::client::handshake(socket).await.unwrap();
            tokio::spawn(async move {
                let _ = conn.await;
            });

            let room = format!("grpc-room-{}", std::process::id());
            let mut join = Vec::new();
            RoomRequest { room: room.clone(), user: "alice".into() }
                .encode(&mut join)
                .unwrap();
            let data = unary(&mut send, "/m7s.RoomService/JoinRoom", join).await;
            assert!(RoomReply::decode(&data[..]).unwrap().ok);

            let mut status = Vec::new();
            RoomRequest { room: room.clone(), user: String::new() }
                .encode(&mut status)
                .unwrap();
            let data = unary(&mut send, "/m7s.RoomService/GetRoom", status).await;
            let status = RoomStatus::decode(&data[..]).unwrap();
            assert_eq!(status.room, room);
            assert_eq!(status.members, 1);

            let mut chat = Vec::new();
            ChatRequest { room: room.clone(), user: "alice".into(), text: "hello".into() }
                .encode(&mut chat)
                .unwrap();
            let data = unary(&mut send, "/m7s.RoomService/SendChat", chat).await;
            let chat = ChatReply::decode(&data[..]).unwrap();
            assert!(chat.ok, "{}", chat.error);
            assert_eq!(chat.text, "hello");

            let mut gift = Vec::new();
            GiftRequest { room: room.clone(), user: "alice".into(), gift: 7, count: 2 }
                .encode(&mut gift)
                .unwrap();
            let data = unary(&mut send, "/m7s.RoomService/SendGift", gift).await;
            assert!(GiftReply::decode(&data[..]).unwrap().total >= 2);
        });
}

async fn unary(send: &mut h2::client::SendRequest<Bytes>, path: &str, payload: Vec<u8>) -> Vec<u8> {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .body(())
        .unwrap();
    let mut framed = vec![0u8];
    framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    framed.extend_from_slice(&payload);
    let (response, mut body) = send.send_request(req, false).unwrap();
    body.send_data(Bytes::from(framed), true).unwrap();
    let response = response.await.unwrap();
    assert_eq!(response.status(), 200);
    let mut data = Vec::new();
    let mut body = response.into_body();
    while let Some(chunk) = body.data().await {
        data.extend_from_slice(&chunk.unwrap());
    }
    assert!(data.len() >= 5, "short gRPC frame");
    data[5..].to_vec()
}
