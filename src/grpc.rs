//! Minimal gRPC server (HTTP/2 + protobuf) for the V6 API surface.
//!
//! Service: `m7s.StreamService/ListStreams`, plus a health check.

use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use h2::server::{self, SendResponse};
use http::{HeaderMap, HeaderValue, Request, Response};
use tokio::net::TcpListener;

use m7s_sdk::StreamApi;

#[derive(Clone, PartialEq, prost::Message)]
pub struct Empty {}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Stream {
    #[prost(string, tag = "1")]
    pub app: String,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(uint64, tag = "3")]
    pub subscribers: u64,
    #[prost(uint64, tag = "4")]
    pub bytes: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Status {
    #[prost(uint64, tag = "1")]
    pub streams: u64,
    #[prost(uint64, tag = "2")]
    pub subscribers: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct GetStreamRequest {
    #[prost(string, tag = "1")]
    pub key: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Plugin {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub module: String,
    #[prost(bool, tag = "3")]
    pub enabled: bool,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct PluginList {
    #[prost(message, repeated, tag = "1")]
    pub plugins: Vec<Plugin>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct StreamList {
    #[prost(message, repeated, tag = "1")]
    pub streams: Vec<Stream>,
}

pub async fn run_with_context(listener: TcpListener, ctx: m7s_sdk::EngineContext) -> Result<()> {
    let streams = ctx.stream_api().ok_or_else(|| anyhow::anyhow!("EngineContext missing streams"))?;
    run(listener, streams).await
}

pub async fn run(listener: TcpListener, streams: Arc<dyn StreamApi>) -> Result<()> {
    log::info!("gRPC listening on {}", listener.local_addr()?);
    loop {
        let (socket, _) = listener.accept().await?;
        let Some(guard) = m7s_sdk::connections::try_acquire() else { continue };
        let streams = streams.clone();
        tokio::spawn(async move {
            let _guard = guard;
            if let Err(e) = serve(socket, streams).await {
                log::debug!("gRPC connection: {e:#}");
            }
        });
    }
}

pub async fn run_tls_with_context(
    listener: TcpListener,
    ctx: m7s_sdk::EngineContext,
    acceptor: tokio_rustls::TlsAcceptor,
) -> Result<()> {
    let streams = ctx.stream_api().ok_or_else(|| anyhow::anyhow!("EngineContext missing streams"))?;
    run_tls(listener, streams, acceptor).await
}

pub async fn run_tls(
    listener: TcpListener,
    streams: Arc<dyn StreamApi>,
    acceptor: tokio_rustls::TlsAcceptor,
) -> Result<()> {
    log::info!("gRPC TLS listening on {}", listener.local_addr()?);
    loop {
        let (socket, _) = listener.accept().await?;
        let Some(guard) = m7s_sdk::connections::try_acquire() else { continue };
        let streams = streams.clone();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            let _guard = guard;
            let Ok(socket) = acceptor.accept(socket).await else { return };
            if let Err(e) = serve(socket, streams).await {
                log::debug!("gRPC TLS connection: {e:#}");
            }
        });
    }
}

async fn serve<S>(socket: S, streams: Arc<dyn StreamApi>) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let mut conn = server::handshake(socket).await?;
    while let Some(request) = conn.accept().await {
        let (request, respond) = request?;
        let streams = streams.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(request, respond, streams).await {
                log::debug!("gRPC request: {e:#}");
            }
        });
    }
    Ok(())
}

async fn handle(
    request: Request<h2::RecvStream>,
    mut respond: SendResponse<Bytes>,
    streams: Arc<dyn StreamApi>,
) -> Result<()> {
    if let Ok(token) = std::env::var("M7S_GRPC_TOKEN") {
        if !token.is_empty() {
            let expected = format!("Bearer {token}");
            let ok = request.headers().get("authorization").and_then(|v| v.to_str().ok()) == Some(expected.as_str());
            if !ok {
                return send_grpc_status(respond, "16").await;
            }
        }
    }
    let path = request.uri().path().to_string();
    let mut body = request.into_body();
    let mut buf = Vec::new();
    while let Some(chunk) = body.data().await {
        buf.extend_from_slice(&chunk?);
    }
    let payload = grpc_payload(&buf);
    if path == "/m7s.StreamService/WatchStreams" {
        let mut response = Response::new(());
        response.headers_mut().insert("content-type", HeaderValue::from_static("application/grpc"));
        *response.status_mut() = http::StatusCode::OK;
        let mut send = respond.send_response(response, false)?;
        for _ in 0..3 {
            let list = StreamList {
                streams: streams.list().into_iter().map(|s| Stream { app: s.app, name: s.name, subscribers: s.subscribers as u64, bytes: s.bytes }).collect(),
            };
            let mut body = Vec::new();
            prost::Message::encode(&list, &mut body)?;
            send.send_data(frame(&body), false)?;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let mut trailers = HeaderMap::new();
        trailers.insert("grpc-status", HeaderValue::from_static("0"));
        send.send_trailers(trailers)?;
        return Ok(());
    }
    let out = match path.as_str() {
        "/m7s.StreamService/ListStreams" => {
            let _: Empty = prost::Message::decode(payload).unwrap_or(Empty {});
            let list = StreamList {
                streams: streams
                    .list()
                    .into_iter()
                    .map(|s| Stream {
                        app: s.app,
                        name: s.name,
                        subscribers: s.subscribers as u64,
                        bytes: s.bytes,
                    })
                    .collect(),
            };
            let mut body = Vec::new();
            prost::Message::encode(&list, &mut body)?;
            frame(&body)
        }
        "/m7s.PluginService/ListPlugins" => {
            let value = crate::plugins::list();
            let list = PluginList {
                plugins: value["plugins"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .map(|p| Plugin {
                                name: p["name"].as_str().unwrap_or("").to_string(),
                                module: p["module"].as_str().unwrap_or("").to_string(),
                                enabled: p["status"].as_str() == Some("enabled"),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            };
            let mut body = Vec::new();
            prost::Message::encode(&list, &mut body)?;
            frame(&body)
        }
        "/m7s.StreamService/GetStream" => {
            let req: GetStreamRequest = prost::Message::decode(payload).unwrap_or(GetStreamRequest { key: String::new() });
            let info = streams.list().into_iter().find(|s| s.key == req.key);
            let stream = info
                .map(|s| Stream {
                    app: s.app,
                    name: s.name,
                    subscribers: s.subscribers as u64,
                    bytes: s.bytes,
                })
                .unwrap_or(Stream { app: String::new(), name: String::new(), subscribers: 0, bytes: 0 });
            let mut body = Vec::new();
            prost::Message::encode(&stream, &mut body)?;
            frame(&body)
        }
        "/m7s.RoomService/ListRooms" => {
            let list = RoomList { rooms: m7s_plugin_live::room_names() };
            let mut body = Vec::new();
            prost::Message::encode(&list, &mut body)?;
            frame(&body)
        }
        "/m7s.RoomService/GetRoom" => {
            let req: RoomRequest = prost::Message::decode(payload)
                .unwrap_or(RoomRequest { room: String::new(), user: String::new() });
            let status = RoomStatus {
                room: req.room.clone(),
                members: m7s_plugin_live::members(&req.room) as u64,
                robots: m7s_plugin_live::robots(&req.room) as u64,
                gifts: m7s_plugin_live::gift_total(&req.room),
                seats: m7s_plugin_live::seats(&req.room).into_iter().flatten().collect(),
            };
            let mut body = Vec::new();
            prost::Message::encode(&status, &mut body)?;
            frame(&body)
        }
        "/m7s.RoomService/SendChat" => {
            let req: ChatRequest = prost::Message::decode(payload).unwrap_or(ChatRequest {
                room: String::new(),
                user: String::new(),
                text: String::new(),
            });
            let reply = match m7s_plugin_live::send_chat(&req.room, &req.user, &req.text) {
                Ok(msg) => ChatReply {
                    ok: true,
                    error: String::new(),
                    user: msg.user,
                    text: msg.text,
                    ts: msg.ts,
                },
                Err(error) => ChatReply {
                    ok: false,
                    error,
                    user: req.user,
                    text: req.text,
                    ts: 0,
                },
            };
            let mut body = Vec::new();
            prost::Message::encode(&reply, &mut body)?;
            frame(&body)
        }
        "/m7s.RoomService/SendGift" => {
            let req: GiftRequest = prost::Message::decode(payload).unwrap_or(GiftRequest {
                room: String::new(),
                user: String::new(),
                gift: 0,
                count: 0,
            });
            let total = m7s_plugin_live::send_gift(&req.room, &req.user, req.gift, req.count);
            let reply = GiftReply { ok: true, total };
            let mut body = Vec::new();
            prost::Message::encode(&reply, &mut body)?;
            frame(&body)
        }
        "/m7s.RoomService/CreateRoom" => {
            let req: RoomRequest = prost::Message::decode(payload).unwrap_or(RoomRequest { room: String::new(), user: String::new() });
            let ok = m7s_plugin_live::create_room(&req.room, &req.user);
            let reply = RoomReply { ok, members: m7s_plugin_live::members(&req.room) as u64 };
            let mut body = Vec::new();
            prost::Message::encode(&reply, &mut body)?;
            frame(&body)
        }
        "/m7s.RoomService/JoinRoom" => {
            let req: RoomRequest = prost::Message::decode(payload).unwrap_or(RoomRequest { room: String::new(), user: String::new() });
            let ok = m7s_plugin_live::join_room(&req.room, &req.user);
            let reply = RoomReply { ok, members: m7s_plugin_live::members(&req.room) as u64 };
            let mut body = Vec::new();
            prost::Message::encode(&reply, &mut body)?;
            frame(&body)
        }
        "/m7s.RoomService/LeaveRoom" => {
            let req: RoomRequest = prost::Message::decode(payload).unwrap_or(RoomRequest { room: String::new(), user: String::new() });
            m7s_plugin_live::leave_room(&req.room, &req.user);
            let reply = RoomReply { ok: true, members: m7s_plugin_live::members(&req.room) as u64 };
            let mut body = Vec::new();
            prost::Message::encode(&reply, &mut body)?;
            frame(&body)
        }
        "/m7s.RoomService/CloseRoom" => {
            let req: RoomRequest = prost::Message::decode(payload).unwrap_or(RoomRequest { room: String::new(), user: String::new() });
            let ok = m7s_plugin_live::close_room(&req.room);
            let reply = RoomReply { ok, members: m7s_plugin_live::members(&req.room) as u64 };
            let mut body = Vec::new();
            prost::Message::encode(&reply, &mut body)?;
            frame(&body)
        }
        "/m7s.StreamService/DeleteStream" => {
            let req: GetStreamRequest = prost::Message::decode(payload).unwrap_or(GetStreamRequest { key: String::new() });
            let (app, name) = req.key.split_once('/').unwrap_or(("", ""));
            let ok = streams.remove(app, name);
            let reply = RoomReply { ok, members: 0 };
            let mut body = Vec::new();
            prost::Message::encode(&reply, &mut body)?;
            frame(&body)
        }
        "/m7s.StreamService/CreateStream" => {
            let req: GetStreamRequest = prost::Message::decode(payload).unwrap_or(GetStreamRequest { key: String::new() });
            let (app, name) = req.key.split_once('/').unwrap_or(("", ""));
            let ok = streams.stream(app, name).is_some();
            let reply = RoomReply { ok, members: 0 };
            let mut body = Vec::new();
            prost::Message::encode(&reply, &mut body)?;
            frame(&body)
        }
        "/m7s.StreamService/GetStatus" => {
            let infos = streams.list();
            let status = Status {
                streams: infos.len() as u64,
                subscribers: infos.iter().map(|s| s.subscribers as u64).sum(),
            };
            let mut body = Vec::new();
            prost::Message::encode(&status, &mut body)?;
            frame(&body)
        }
        "/grpc.health.v1.Health/Check" => {
            let mut body = Vec::new();
            prost::Message::encode(&Empty {}, &mut body)?;
            frame(&body)
        }
        _ => {
            let mut response = Response::new(());
            response.headers_mut().insert(
                "content-type",
                HeaderValue::from_static("application/grpc"),
            );
            *response.status_mut() = http::StatusCode::OK;
            let mut send = respond.send_response(response, false)?;
            let mut trailers = HeaderMap::new();
            trailers.insert("grpc-status", HeaderValue::from_static("12"));
            send.send_trailers(trailers)?;
            return Ok(());
        }
    };

    let mut response = Response::new(());
    response
        .headers_mut()
        .insert("content-type", HeaderValue::from_static("application/grpc"));
    *response.status_mut() = http::StatusCode::OK;
    let mut send = respond.send_response(response, false)?;
    send.send_data(out, false)?;
    let mut trailers = HeaderMap::new();
    trailers.insert("grpc-status", HeaderValue::from_static("0"));
    send.send_trailers(trailers)?;
    Ok(())
}

fn grpc_payload(buf: &[u8]) -> &[u8] {
    if buf.len() >= 5 {
        let len = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
        if 5 + len <= buf.len() {
            return &buf[5..5 + len];
        }
    }
    &[]
}

fn frame(payload: &[u8]) -> Bytes {
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(0);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    Bytes::from(out)
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct RoomStatus {
    #[prost(string, tag = "1")]
    pub room: String,
    #[prost(uint64, tag = "2")]
    pub members: u64,
    #[prost(uint64, tag = "3")]
    pub robots: u64,
    #[prost(uint64, tag = "4")]
    pub gifts: u64,
    #[prost(string, repeated, tag = "5")]
    pub seats: Vec<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ChatRequest {
    #[prost(string, tag = "1")]
    pub room: String,
    #[prost(string, tag = "2")]
    pub user: String,
    #[prost(string, tag = "3")]
    pub text: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ChatReply {
    #[prost(bool, tag = "1")]
    pub ok: bool,
    #[prost(string, tag = "2")]
    pub error: String,
    #[prost(string, tag = "3")]
    pub user: String,
    #[prost(string, tag = "4")]
    pub text: String,
    #[prost(uint64, tag = "5")]
    pub ts: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct GiftRequest {
    #[prost(string, tag = "1")]
    pub room: String,
    #[prost(string, tag = "2")]
    pub user: String,
    #[prost(uint32, tag = "3")]
    pub gift: u32,
    #[prost(uint32, tag = "4")]
    pub count: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct GiftReply {
    #[prost(bool, tag = "1")]
    pub ok: bool,
    #[prost(uint64, tag = "2")]
    pub total: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct RoomList {
    #[prost(string, repeated, tag = "1")]
    pub rooms: Vec<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct RoomRequest {
    #[prost(string, tag = "1")]
    pub room: String,
    #[prost(string, tag = "2")]
    pub user: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct RoomReply {
    #[prost(bool, tag = "1")]
    pub ok: bool,
    #[prost(uint64, tag = "2")]
    pub members: u64,
}

async fn send_grpc_status(mut respond: SendResponse<Bytes>, status: &'static str) -> Result<()> {
    let mut response = Response::new(());
    response
        .headers_mut()
        .insert("content-type", HeaderValue::from_static("application/grpc"));
    *response.status_mut() = http::StatusCode::OK;
    let mut send = respond.send_response(response, false)?;
    let mut trailers = HeaderMap::new();
    trailers.insert("grpc-status", HeaderValue::from_static(status));
    send.send_trailers(trailers)?;
    Ok(())
}
