# 详细设计 — m7s-rs

## 1. Architecture

```text
protocol sources ──► Engine StreamManager ──► Stream
                         │                    ├─ RingBuffer<Arc<FlvTag>> (SPMC)
                         │                    ├─ GOP cache
                         │                    └─ tap broadcast (compat)
                         ▼
                 protocol egress / plugins
```

- `m7s-sdk` defines `Plugin`, `EngineContext`, `TagSink`, `TagReader`,
  `TagStream`, `StreamApi` (`stream/get/list/count`), `PluginRuntime`.
- Engine `Stream` implements all of them; plugins never depend on engine types.
- 26 plugin crates: 10 protocol, 6 media, 5 utility, 3 device, 2 room.

## 2. Stream data plane

- `RingBuffer<T>`: `ArcSwapOption` slot + `AtomicU64` sequence; readers use
  `try_next()`, writers `push()`. No lock on the write path.
- `Stream::push` updates GOP cache, ring, and an optional tap broadcast.
- `TagStream::snapshot()` returns GOP/config for SDP/fMP4.
- `TagStream::tap()` exists only for WHEP/WHIP compatibility; zero cost with no
  receivers.

## 3. Protocol paths

| Path | Ingest | Egress |
|---|---|---|
| FLV/RTMP | RTMP chunk/AMF0 | HTTP-FLV stream |
| HLS TS | — | Segmenter -> `.ts` + playlist |
| HLS fMP4 | — | `/fmp4/...` init/moof/mdat, PART |
| RTSP | — | TCP interleaved / UDP RTP |
| SRT | MPEG-TS demux | — |
| WebTransport | datagram + bi stream TS | — |
| GB28181 | SIP, PS/RTP demux | Playback PS/RTP sender |
| WHEP/WHIP | WHIP RTP | WHEP RTP |
| Cluster | HTTP/QUIC relay | HTTP-FLV |

## 4. HLS fMP4

- Init: `ftyp + moov` with `mvhd`, one video `trak`, `mvex/trex`.
- Fragment: `moof(mfhd, traf(tfhd, tfdt, trun)) + mdat`.
- `trun` uses data-offset patched after moof length is known.
- Parts: samples split at ~0.5s, each part is its own `moof+mdat`.
- Playlist: `EXT-X-MAP`, `EXT-X-PART-INF`, `EXT-X-PART`, `EXTINF`.

## 5. Security

- API token via `M7S_API_TOKEN` / config.
- JWT HS256 (`M7S_JWT_SECRET`), `/api/login`.
- File users (`data/users.json`) or MySQL (`M7S_MYSQL_URL`).
- Admin UI stores token in localStorage and sends Bearer.
- TLS via rustls for HTTPS/RTMPS; mTLS test coverage.

## 6. Cluster

- `/cluster/api/sessions`, `/servers`, `/status`, `/relays`.
- Relay lookup: HTTP sessions + QUIC edge; local pull via HTTP-FLV.
- `ensure_relay` uses non-creating `get` (bug fixed), then creates stream.

## 7. Observability

- `/health`, `/metrics` (Prometheus text), `/api/streams`.
- Per-stream `bytes`, `cached_tags`, `subscribers`, latency samples.
- gRPC `ListStreams`, `GetStatus`, health.

## 8. Failure handling

- Ring reader is lock-free; slow consumers drop independently.
- Tap lag is handled by `broadcast::Lagged`.
- Plugin listener errors are logged, process continues.
- Seat limit returns `None` from `try_get_or_create`.

## 9. Deployment

Single binary, config `config/server.toml`. Optional MySQL. Ports:
8180 HTTP, 1935 RTMP, 8554 RTSP, 8081/8082 WHEP/WHIP, 9000 SRT,
4443 WebTransport, 50051 gRPC, 9443 QUIC cluster.

## 10. Test design

- Unit: ring, stream, plugin registry, JWT, HLS/fMP4 boxes, PS/RTP, ONVIF/GB.
- Integration: RTMP publish, RTSP play, HLS HTTP, cluster relay, gRPC h2,
  Web SDK assets, fMP4 playlist.
- Load: `scripts/load_test.cjs` 10K HTTP-FLV.
- Browser: Chrome WHEP A/V, WHIP canvas push.
