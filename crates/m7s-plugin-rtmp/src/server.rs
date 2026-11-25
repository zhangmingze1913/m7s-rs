//! Minimal RTMP server: handshake, connect, createStream, publish, play.

use std::sync::Arc;

use anyhow::{Result, bail};
use tokio::io::{AsyncRead, AsyncWrite, WriteHalf, split};
use tokio::net::TcpListener;

use tokio_rustls::TlsAcceptor;

use m7s_codec::FlvTag;
use m7s_sdk::{StreamApi, TagStream};
use crate::amf0::{self, Amf0};
use crate::chunk::{self, ChunkReader, ChunkWriter, RtmpMessage};

pub async fn run(
    listener: TcpListener,
    streams: Arc<dyn StreamApi>,
    cfg: RtmpConfig,
    recorder: Option<Arc<dyn RecordHook>>,
) -> Result<()> {
    log::info!("RTMP listening on {}", listener.local_addr()?);
    loop {
        let (socket, peer) = listener.accept().await?;
        let Some(guard) = m7s_sdk::connections::try_acquire() else {
            log::warn!("RTMP connection limit reached");
            drop(socket);
            continue;
        };
        if !m7s_sdk::plugins::enabled("rtmp") {
            drop(socket);
            continue;
        }
        socket.set_nodelay(true).ok();
        let streams = streams.clone();
        let cfg = cfg.clone();
        let recorder = recorder.clone();
        tokio::spawn(async move {
            let _guard = guard;
            if let Err(e) = handle(socket, streams, cfg, recorder).await {
                log::debug!("RTMP client {peer}: {e:#}");
            }
        });
    }
}

pub async fn run_tls(
    listener: TcpListener,
    streams: Arc<dyn StreamApi>,
    cfg: RtmpConfig,
    recorder: Option<Arc<dyn RecordHook>>,
    acceptor: TlsAcceptor,
) -> Result<()> {
    log::info!("RTMPS listening on {}", listener.local_addr()?);
    loop {
        let (socket, peer) = listener.accept().await?;
        let Some(guard) = m7s_sdk::connections::try_acquire() else {
            log::warn!("RTMPS connection limit reached");
            drop(socket);
            continue;
        };
        if !m7s_sdk::plugins::enabled("rtmp") {
            drop(socket);
            continue;
        }
        socket.set_nodelay(true).ok();
        let streams = streams.clone();
        let cfg = cfg.clone();
        let recorder = recorder.clone();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            let _guard = guard;
            match acceptor.accept(socket).await {
                Ok(tls) => {
                    if let Err(e) = handle(tls, streams, cfg, recorder).await {
                        log::debug!("RTMPS client {peer}: {e:#}");
                    }
                }
                Err(e) => log::debug!("RTMPS handshake {peer}: {e}"),
            }
        });
    }
}

async fn handle<S>(
    socket: S,
    streams: Arc<dyn StreamApi>,
    cfg: RtmpConfig,
    recorder: Option<Arc<dyn RecordHook>>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut socket = socket;
    chunk::server_handshake(&mut socket).await?;
    let (mut reader, mut writer) = split(socket);
    let mut chunks = ChunkReader::new();
    let mut out = ChunkWriter::new();
    let mut app = "live".to_string();
    let mut publishing: Option<Arc<dyn TagStream>> = None;

    loop {
        let msg = chunks.read_message(&mut reader).await?;
        match msg.type_id {
            1 => {
                if msg.payload.len() >= 4 {
                    let size = u32::from_be_bytes(msg.payload[..4].try_into().unwrap()) as usize;
                    chunks.set_chunk_size(size);
                    out.set_chunk_size(size);
                }
            }
            20 | 17 => {
                let data = if msg.type_id == 17 && !msg.payload.is_empty() {
                    &msg.payload[1..]
                } else {
                    &msg.payload
                };
                let values = amf0::decode_all(data)?;
                let Some(command) = values.first().and_then(Amf0::as_str) else {
                    continue;
                };
                let tx = values.get(1).and_then(Amf0::as_f64).unwrap_or(0.0);
                match command {
                    "connect" => {
                        if let Some(obj) = values.get(2) {
                            if let Some(a) = obj.get("app").and_then(Amf0::as_str) {
                                app = a.to_string();
                            }
                            if let Some(tc) = obj.get("tcUrl").and_then(Amf0::as_str) {
                                if !authorized(&cfg, tc) {
                                    bail!("publish key rejected");
                                }
                            }
                        }
                        send_control(
                            &mut out,
                            &mut writer,
                            5,
                            2_500_000u32.to_be_bytes().to_vec(),
                        )
                        .await?;
                        send_control(
                            &mut out,
                            &mut writer,
                            6,
                            [&2_500_000u32.to_be_bytes()[..], &[2][..]].concat(),
                        )
                        .await?;
                        send_control(&mut out, &mut writer, 1, 4096u32.to_be_bytes().to_vec())
                            .await?;
                        chunks.set_chunk_size(4096);
                        out.set_chunk_size(4096);
                        let props = Amf0::Object(vec![
                            ("fmsVer".into(), Amf0::String("FMS/3,0,1,123".into())),
                            ("capabilities".into(), Amf0::Number(31.0)),
                        ]);
                        let info = Amf0::Object(vec![
                            ("level".into(), Amf0::String("status".into())),
                            (
                                "code".into(),
                                Amf0::String("NetConnection.Connect.Success".into()),
                            ),
                            (
                                "description".into(),
                                Amf0::String("Connection succeeded.".into()),
                            ),
                            ("objectEncoding".into(), Amf0::Number(0.0)),
                        ]);
                        write_command(&mut out, &mut writer, "_result", tx, vec![props, info])
                            .await?;
                    }
                    "createStream" => {
                        write_command(
                            &mut out,
                            &mut writer,
                            "_result",
                            tx,
                            vec![Amf0::Null, Amf0::Number(1.0)],
                        )
                        .await?;
                    }
                    "publish" => {
                        let name = values
                            .get(3)
                            .and_then(Amf0::as_str)
                            .unwrap_or("stream")
                            .to_string();
                        let (name, key) = split_query(&name);
                        if !authorized(&cfg, &key) {
                            bail!("publish key rejected");
                        }
                        let Some(stream) = streams.stream(&app, &name) else {
                            bail!("stream seat limit exceeded");
                        };
                        publishing = Some(stream);
                        if let Some(recorder) = &recorder { recorder.ensure(&app, &name); }
                        write_on_status(
                            &mut out,
                            &mut writer,
                            "NetStream.Publish.Start",
                            "Publishing started.",
                        )
                        .await?;
                    }
                    "play" => {
                        let name = values
                            .get(3)
                            .and_then(Amf0::as_str)
                            .unwrap_or("stream")
                            .to_string();
                        let (name, _) = split_query(&name);
                        let Some(stream) = streams.stream(&app, &name) else {
                            bail!("stream seat limit exceeded");
                        };
                        write_on_status(&mut out, &mut writer, "NetStream.Play.Reset", "Playing.")
                            .await?;
                        write_on_status(&mut out, &mut writer, "NetStream.Play.Start", "Playing.")
                            .await?;
                        let cache: Vec<Arc<FlvTag>> = stream
                            .snapshot()
                            .into_iter()
                            .map(|t| Arc::new(FlvTag::new(t.tag_type, t.timestamp, t.data)))
                            .collect();
                        let reader = stream.tag_reader();
                        tokio::spawn(play_loop(stream, cache, reader, writer, 4096));
                        return Ok(());
                    }
                    "deleteStream" | "closeStream" => return Ok(()),
                    _ => {}
                }
            }
            8 | 9 | 18 => {
                if let Some(stream) = &publishing {
                    stream.push_tag(msg.type_id, msg.timestamp, msg.payload);
                }
            }
            _ => {}
        }
    }
}

async fn play_loop<S>(
    stream: Arc<dyn TagStream>,
    cache: Vec<Arc<FlvTag>>,
    mut reader: Box<dyn m7s_sdk::TagReader>,
    mut writer: WriteHalf<S>,
    chunk_size: usize,
) where
    S: AsyncWrite + Unpin + Send + 'static,
{
    let mut out = ChunkWriter::new();
    out.set_chunk_size(chunk_size);
    if let Some(metadata) = stream.metadata() {
        let msg = RtmpMessage {
            type_id: 18,
            stream_id: 1,
            timestamp: metadata.timestamp,
            payload: metadata.data,
        };
        if out.write_message(&mut writer, &msg).await.is_err() {
            stream.leave();
            return;
        }
    }
    for tag in cache {
        if send_tag(&mut out, &mut writer, &tag).await.is_err() {
            stream.leave();
            return;
        }
    }
    loop {
        match reader.try_next() {
            Some(t) => {
                let tag = FlvTag::new(t.tag_type, t.timestamp, t.data);
                if send_tag(&mut out, &mut writer, &tag).await.is_err() {
                    break;
                }
            }
            None => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
        }
    }
    stream.leave();
}

async fn send_tag<S>(out: &mut ChunkWriter, writer: &mut WriteHalf<S>, tag: &FlvTag) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let msg = RtmpMessage {
        type_id: tag.tag_type,
        stream_id: 1,
        timestamp: tag.timestamp,
        payload: tag.data.clone(),
    };
    out.write_message(writer, &msg).await
}

async fn send_control<S>(
    out: &mut ChunkWriter,
    writer: &mut WriteHalf<S>,
    type_id: u8,
    payload: Vec<u8>,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    out.write_message(
        writer,
        &RtmpMessage {
            type_id,
            stream_id: 0,
            timestamp: 0,
            payload,
        },
    )
    .await
}

async fn write_command<S>(
    out: &mut ChunkWriter,
    writer: &mut WriteHalf<S>,
    name: &str,
    tx: f64,
    args: Vec<Amf0>,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let mut values = vec![Amf0::String(name.to_string()), Amf0::Number(tx)];
    values.extend(args);
    let mut payload = Vec::new();
    for value in &values {
        payload.extend_from_slice(&amf0::encode(value));
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
}

async fn write_on_status<S>(
    out: &mut ChunkWriter,
    writer: &mut WriteHalf<S>,
    code: &str,
    description: &str,
) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    write_command(
        out,
        writer,
        "onStatus",
        0.0,
        vec![
            Amf0::Null,
            Amf0::Object(vec![
                ("level".into(), Amf0::String("status".into())),
                ("code".into(), Amf0::String(code.into())),
                ("description".into(), Amf0::String(description.into())),
            ]),
        ],
    )
    .await
}

fn split_query(value: &str) -> (String, String) {
    match value.split_once('?') {
        Some((name, query)) => {
            let key = query
                .split('&')
                .find_map(|p| p.strip_prefix("key="))
                .unwrap_or("")
                .to_string();
            (name.to_string(), key)
        }
        None => (value.to_string(), String::new()),
    }
}

fn authorized(cfg: &RtmpConfig, key: &str) -> bool {
    cfg.publish_key.is_empty() || key == cfg.publish_key
}

#[derive(Clone, Default)]
pub struct RtmpConfig {
    pub publish_key: String,
}

pub trait RecordHook: Send + Sync {
    fn ensure(&self, app: &str, name: &str);
}
