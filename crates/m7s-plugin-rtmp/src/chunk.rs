//! RTMP chunk stream and handshake.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, Clone)]
pub struct RtmpMessage {
    pub type_id: u8,
    pub stream_id: u32,
    pub timestamp: u32,
    pub payload: Vec<u8>,
}

#[derive(Default)]
struct ChunkState {
    timestamp: u32,
    length: u32,
    type_id: u8,
    stream_id: u32,
    received: Vec<u8>,
    in_progress: bool,
}

pub struct ChunkReader {
    chunk_size: usize,
    states: HashMap<u32, ChunkState>,
}

impl Default for ChunkReader {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkReader {
    pub fn new() -> Self {
        Self {
            chunk_size: 128,
            states: HashMap::new(),
        }
    }

    pub fn set_chunk_size(&mut self, size: usize) {
        self.chunk_size = size.clamp(1, 65536);
    }

    pub async fn read_message<R: AsyncRead + Unpin>(
        &mut self,
        reader: &mut R,
    ) -> Result<RtmpMessage> {
        loop {
            let b0 = reader.read_u8().await.context("read RTMP basic header")?;
            let fmt = b0 >> 6;
            let mut csid = (b0 & 0x3f) as u32;
            if csid == 0 {
                csid = reader.read_u8().await? as u32 + 64;
            } else if csid == 1 {
                let mut two = [0u8; 2];
                reader.read_exact(&mut two).await?;
                csid = u16::from_le_bytes(two) as u32 + 64;
            }
            let state = self.states.entry(csid).or_default();
            let mut extended = false;
            match fmt {
                0 => {
                    let ts = read_u24(reader).await?;
                    let len = read_u24(reader).await?;
                    let type_id = reader.read_u8().await?;
                    let mut sid = [0u8; 4];
                    reader.read_exact(&mut sid).await?;
                    state.timestamp = ts;
                    state.length = len;
                    state.type_id = type_id;
                    state.stream_id = u32::from_le_bytes(sid);
                    state.received.clear();
                    state.in_progress = true;
                    extended = ts == 0x00ff_ffff;
                }
                1 => {
                    let delta = read_u24(reader).await?;
                    let len = read_u24(reader).await?;
                    let type_id = reader.read_u8().await?;
                    state.timestamp = state.timestamp.wrapping_add(delta);
                    state.length = len;
                    state.type_id = type_id;
                    state.received.clear();
                    state.in_progress = true;
                    extended = delta == 0x00ff_ffff;
                }
                2 => {
                    let delta = read_u24(reader).await?;
                    state.timestamp = state.timestamp.wrapping_add(delta);
                    state.received.clear();
                    state.in_progress = true;
                    extended = delta == 0x00ff_ffff;
                }
                3 => {}
                _ => unreachable!(),
            }
            if extended {
                let mut ext = [0u8; 4];
                reader.read_exact(&mut ext).await?;
                state.timestamp = u32::from_be_bytes(ext);
            }
            if !state.in_progress {
                state.received.clear();
                state.in_progress = true;
            }
            let remaining = state.length as usize - state.received.len();
            let take = remaining.min(self.chunk_size);
            let start = state.received.len();
            state.received.resize(start + take, 0);
            reader.read_exact(&mut state.received[start..]).await?;
            if state.received.len() >= state.length as usize {
                let message = RtmpMessage {
                    type_id: state.type_id,
                    stream_id: state.stream_id,
                    timestamp: state.timestamp,
                    payload: std::mem::take(&mut state.received),
                };
                state.in_progress = false;
                return Ok(message);
            }
        }
    }
}

async fn read_u24<R: AsyncRead + Unpin>(reader: &mut R) -> Result<u32> {
    let mut b = [0u8; 3];
    reader.read_exact(&mut b).await?;
    Ok(u32::from_be_bytes([0, b[0], b[1], b[2]]))
}

pub struct ChunkWriter {
    chunk_size: usize,
}

impl Default for ChunkWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkWriter {
    pub fn new() -> Self {
        Self { chunk_size: 128 }
    }

    pub fn set_chunk_size(&mut self, size: usize) {
        self.chunk_size = size.clamp(1, 65536);
    }

    pub async fn write_message<W: AsyncWrite + Unpin>(
        &mut self,
        writer: &mut W,
        msg: &RtmpMessage,
    ) -> Result<()> {
        if msg.payload.len() > 0x00ff_ffff {
            bail!("RTMP message too large");
        }
        let mut header = Vec::with_capacity(12);
        header.push(0x03); // fmt 0, csid 3
        push_u24(&mut header, msg.timestamp.min(0x00ff_ffff));
        push_u24(&mut header, msg.payload.len() as u32);
        header.push(msg.type_id);
        header.extend_from_slice(&msg.stream_id.to_le_bytes());
        writer.write_all(&header).await?;
        let mut offset = 0;
        while offset < msg.payload.len() {
            if offset > 0 {
                writer.write_all(&[0xc3]).await?; // fmt 3, csid 3
            }
            let end = (offset + self.chunk_size).min(msg.payload.len());
            writer.write_all(&msg.payload[offset..end]).await?;
            offset = end;
        }
        writer.flush().await?;
        Ok(())
    }
}

fn push_u24(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes()[1..]);
}

pub async fn server_handshake<S>(stream: &mut S) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut c0 = [0u8; 1];
    stream.read_exact(&mut c0).await.context("RTMP C0")?;
    if c0[0] != 3 {
        bail!("unsupported RTMP version {}", c0[0]);
    }
    let mut c1 = [0u8; 1536];
    stream.read_exact(&mut c1).await?;
    let mut s0s1s2 = Vec::with_capacity(1 + 1536 + 1536);
    s0s1s2.push(3);
    let mut s1 = [0u8; 1536];
    s1[..8].copy_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
    s0s1s2.extend_from_slice(&s1);
    s0s1s2.extend_from_slice(&c1);
    stream.write_all(&s0s1s2).await?;
    stream.flush().await?;
    let mut c2 = [0u8; 1536];
    stream.read_exact(&mut c2).await?;
    Ok(())
}

pub async fn client_handshake<S>(stream: &mut S) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut c0c1 = vec![3u8];
    c0c1.extend_from_slice(&[0u8; 1536]);
    stream.write_all(&c0c1).await?;
    let mut s0 = [0u8; 1];
    stream.read_exact(&mut s0).await?;
    if s0[0] != 3 {
        bail!("bad S0");
    }
    let mut s1s2 = [0u8; 3072];
    stream.read_exact(&mut s1s2).await?;
    stream.write_all(&s1s2[..1536]).await?;
    stream.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn fragmented_message_roundtrip() {
        runtime().block_on(async {
            let (mut a, mut b) = tokio::io::duplex(64);
            let msg = RtmpMessage {
                type_id: 9,
                stream_id: 1,
                timestamp: 123,
                payload: vec![7u8; 400],
            };
            let expected = msg.clone();
            let writer = tokio::spawn(async move {
                let mut w = ChunkWriter::new();
                w.set_chunk_size(128);
                w.write_message(&mut a, &msg).await.unwrap();
            });
            let mut r = ChunkReader::new();
            let got = r.read_message(&mut b).await.unwrap();
            writer.await.unwrap();
            assert_eq!(got.type_id, expected.type_id);
            assert_eq!(got.timestamp, expected.timestamp);
            assert_eq!(got.payload, expected.payload);
        });
    }
}
