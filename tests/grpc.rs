
use bytes::Bytes;
use http::Request;
use m7s_rs::grpc::{StreamList, run as grpc_run};
use prost::Message;
use m7s_rs::stream::StreamManager;
use tokio::net::{TcpListener, TcpStream};

#[test]
fn grpc_list_streams() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let streams = StreamManager::new(10);
            streams.get_or_create("live", "grpc");
            tokio::spawn(grpc_run(listener, streams));

            let socket = TcpStream::connect(addr).await.unwrap();
            let (mut send_request, conn) = h2::client::handshake(socket).await.unwrap();
            tokio::spawn(async move {
                let _ = conn.await;
            });

            let mut req = Request::builder()
                .method("POST")
                .uri("/m7s.StreamService/ListStreams")
                .header("content-type", "application/grpc")
                .header("te", "trailers")
                .body(())
                .unwrap();
            let _ = &mut req;
            let (response, mut body) = send_request.send_request(req, false).unwrap();
            body.send_data(Bytes::from_static(&[0, 0, 0, 0, 0]), true)
                .unwrap();
            let response = response.await.unwrap();
            assert_eq!(response.status(), 200);
            let mut resp_body = response.into_body();
            let mut data = Vec::new();
            while let Some(chunk) = resp_body.data().await {
                data.extend_from_slice(&chunk.unwrap());
            }
            assert!(data.len() > 5);
            let list = StreamList::decode(&data[5..]).unwrap();
            assert!(
                list.streams
                    .iter()
                    .any(|s| s.app == "live" && s.name == "grpc")
            );

            let get = m7s_rs::grpc::GetStreamRequest { key: "live/grpc".to_string() };
            let mut payload = Vec::new();
            get.encode(&mut payload).unwrap();
            let mut framed = vec![0u8];
            framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            framed.extend_from_slice(&payload);
            let req = Request::builder()
                .method("POST")
                .uri("/m7s.StreamService/GetStream")
                .header("content-type", "application/grpc")
                .header("te", "trailers")
                .body(())
                .unwrap();
            let (response, mut body) = send_request.send_request(req, false).unwrap();
            body.send_data(Bytes::from(framed), true).unwrap();
            let response = response.await.unwrap();
            let mut resp_body = response.into_body();
            let mut data = Vec::new();
            while let Some(chunk) = resp_body.data().await {
                data.extend_from_slice(&chunk.unwrap());
            }
            let got = m7s_rs::grpc::Stream::decode(&data[5..]).unwrap();
            assert_eq!(got.name, "grpc");

            let req = Request::builder()
                .method("POST")
                .uri("/m7s.StreamService/GetStatus")
                .header("content-type", "application/grpc")
                .header("te", "trailers")
                .body(())
                .unwrap();
            let (response, mut body) = send_request.send_request(req, false).unwrap();
            body.send_data(Bytes::from_static(&[0, 0, 0, 0, 0]), true)
                .unwrap();
            let response = response.await.unwrap();
            let mut resp_body = response.into_body();
            let mut data = Vec::new();
            while let Some(chunk) = resp_body.data().await {
                data.extend_from_slice(&chunk.unwrap());
            }
            let status = m7s_rs::grpc::Status::decode(&data[5..]).unwrap();
            assert!(status.streams >= 1);

            let req = Request::builder()
                .method("POST")
                .uri("/m7s.PluginService/ListPlugins")
                .header("content-type", "application/grpc")
                .header("te", "trailers")
                .body(())
                .unwrap();
            let (response, mut body) = send_request.send_request(req, false).unwrap();
            body.send_data(Bytes::from_static(&[0, 0, 0, 0, 0]), true).unwrap();
            let response = response.await.unwrap();
            let mut resp_body = response.into_body();
            let mut data = Vec::new();
            while let Some(chunk) = resp_body.data().await {
                data.extend_from_slice(&chunk.unwrap());
            }
            let del = m7s_rs::grpc::GetStreamRequest { key: "live/grpc".to_string() };
            let mut payload = Vec::new();
            del.encode(&mut payload).unwrap();
            let mut framed = vec![0u8];
            framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            framed.extend_from_slice(&payload);
            let req = Request::builder()
                .method("POST")
                .uri("/m7s.StreamService/DeleteStream")
                .header("content-type", "application/grpc")
                .header("te", "trailers")
                .body(())
                .unwrap();
            let (response, mut body) = send_request.send_request(req, false).unwrap();
            body.send_data(Bytes::from(framed), true).unwrap();
            let response = response.await.unwrap();
            let mut resp_body = response.into_body();
            let mut del_data = Vec::new();
            while let Some(chunk) = resp_body.data().await {
                del_data.extend_from_slice(&chunk.unwrap());
            }
            let reply = m7s_rs::grpc::RoomReply::decode(&del_data[5..]).unwrap();
            assert!(reply.ok, "delete stream failed");
            let plugins = m7s_rs::grpc::PluginList::decode(&data[5..]).unwrap();
            let create = m7s_rs::grpc::GetStreamRequest { key: "live/new".to_string() };
            let mut payload = Vec::new();
            create.encode(&mut payload).unwrap();
            let mut framed = vec![0u8];
            framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            framed.extend_from_slice(&payload);
            let req = Request::builder()
                .method("POST")
                .uri("/m7s.StreamService/CreateStream")
                .header("content-type", "application/grpc")
                .header("te", "trailers")
                .body(())
                .unwrap();
            let (response, mut body) = send_request.send_request(req, false).unwrap();
            body.send_data(Bytes::from(framed), true).unwrap();
            let response = response.await.unwrap();
            let mut resp_body = response.into_body();
            let mut cdata = Vec::new();
            while let Some(chunk) = resp_body.data().await {
                cdata.extend_from_slice(&chunk.unwrap());
            }
            let reply = m7s_rs::grpc::RoomReply::decode(&cdata[5..]).unwrap();
            assert!(reply.ok, "create stream failed");
            assert_eq!(plugins.plugins.len(), 26);
            assert!(plugins.plugins.iter().any(|p| p.name == "rtmp" && p.enabled));

            let room = m7s_rs::grpc::RoomRequest { room: "grpc-room".to_string(), user: "u".to_string() };
            let mut payload = Vec::new();
            room.encode(&mut payload).unwrap();
            let mut framed = vec![0u8];
            framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            framed.extend_from_slice(&payload);
            let create_req = Request::builder()
                .method("POST")
                .uri("/m7s.RoomService/CreateRoom")
                .header("content-type", "application/grpc")
                .header("te", "trailers")
                .body(())
                .unwrap();
            let (response, mut body) = send_request.send_request(create_req, false).unwrap();
            body.send_data(Bytes::from(framed.clone()), true).unwrap();
            let response = response.await.unwrap();
            let mut resp_body = response.into_body();
            let mut data = Vec::new();
            while let Some(chunk) = resp_body.data().await {
                data.extend_from_slice(&chunk.unwrap());
            }
            let reply = m7s_rs::grpc::RoomReply::decode(&data[5..]).unwrap();
            assert!(reply.ok, "create room failed");
            let req = Request::builder()
                .method("POST")
                .uri("/m7s.RoomService/JoinRoom")
                .header("content-type", "application/grpc")
                .header("te", "trailers")
                .body(())
                .unwrap();
            let (response, mut body) = send_request.send_request(req, false).unwrap();
            body.send_data(Bytes::from(framed), true).unwrap();
            let response = response.await.unwrap();
            let mut resp_body = response.into_body();
            let mut data = Vec::new();
            while let Some(chunk) = resp_body.data().await {
                data.extend_from_slice(&chunk.unwrap());
            }
            let reply = m7s_rs::grpc::RoomReply::decode(&data[5..]).unwrap();
            assert!(reply.ok);
            assert_eq!(reply.members, 1);

            let req = Request::builder()
                .method("POST")
                .uri("/m7s.RoomService/ListRooms")
                .header("content-type", "application/grpc")
                .header("te", "trailers")
                .body(())
                .unwrap();
            let (response, mut body) = send_request.send_request(req, false).unwrap();
            body.send_data(Bytes::from_static(&[0, 0, 0, 0, 0]), true).unwrap();
            let response = response.await.unwrap();
            let mut resp_body = response.into_body();
            let mut data = Vec::new();
            while let Some(chunk) = resp_body.data().await {
                data.extend_from_slice(&chunk.unwrap());
            }
            let rooms = m7s_rs::grpc::RoomList::decode(&data[5..]).unwrap();
            assert!(rooms.rooms.iter().any(|r| r == "grpc-room"));
            let req = Request::builder()
                .method("POST")
                .uri("/m7s.RoomService/CloseRoom")
                .header("content-type", "application/grpc")
                .header("te", "trailers")
                .body(())
                .unwrap();
            let mut payload = Vec::new();
            room.encode(&mut payload).unwrap();
            let mut framed = vec![0u8];
            framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            framed.extend_from_slice(&payload);
            let (response, mut body) = send_request.send_request(req, false).unwrap();
            body.send_data(Bytes::from(framed), true).unwrap();
            let response = response.await.unwrap();
            let mut resp_body = response.into_body();
            let mut data = Vec::new();
            while let Some(chunk) = resp_body.data().await {
                data.extend_from_slice(&chunk.unwrap());
            }
            let reply = m7s_rs::grpc::RoomReply::decode(&data[5..]).unwrap();
            assert!(reply.ok, "close room failed");
            assert!(!m7s_plugin_live::close_room("grpc-room"), "already closed");
        });
}

#[test]
fn grpc_watch_streams() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let streams = StreamManager::new(10);
            streams.get_or_create("live", "watch");
            tokio::spawn(grpc_run(listener, streams));

            let socket = TcpStream::connect(addr).await.unwrap();
            let (mut send_request, conn) = h2::client::handshake(socket).await.unwrap();
            tokio::spawn(async move { let _ = conn.await; });
            let req = Request::builder()
                .method("POST")
                .uri("/m7s.StreamService/WatchStreams")
                .header("content-type", "application/grpc")
                .header("te", "trailers")
                .body(())
                .unwrap();
            let (response, mut body) = send_request.send_request(req, false).unwrap();
            body.send_data(Bytes::from_static(&[0, 0, 0, 0, 0]), true).unwrap();
            let response = response.await.unwrap();
            let mut resp = response.into_body();
            let mut frames = 0;
            while let Some(chunk) = resp.data().await {
                let chunk = chunk.unwrap();
                if chunk.len() >= 5 && chunk[0] == 0 {
                    frames += 1;
                }
            }
            assert!(frames >= 3, "frames={frames}");
            let trailers = resp.trailers().await.unwrap().unwrap();
            assert_eq!(trailers.get("grpc-status").unwrap(), "0");
        });
}
