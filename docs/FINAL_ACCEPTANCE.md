# Final Acceptance Snapshot (Monibuca V6 target)

Date: 2026-11-16

## Performance

| Test | Result |
|---|---|
| HTTP-FLV 10,000 viewers (release, demo) | **10000/10000, 15.12s** |
| Ring write, pooled Arc | **117.2 ns/write** |
| Ring write, allocating | 566.9 ns/write |

## Protocol evidence

| Protocol | Evidence |
|---|---|
| RTMP | `rtmp_publish` integration |
| HTTP-FLV | 10K load, FLV magic |
| HLS | `hls_http`, `#EXTM3U`, TS segments |
| RTSP | `rtsp_describe_setup_play`; real AAC pull exit 0 |
| SRT | FFmpeg push, `live/srt bytes=165587` |
| WHEP | Chrome A/V `frames=206`, audio time 7.9 |
| WHIP | `live/whip bytes=113270` |
| WebTransport | datagram `81091`; reliable stream `81091` |
| GB28181 | 12 SIP/PS/RTP tests |
| ONVIF | Device/Media/Discovery + WS-Security |
| MP4 | `ftyp`, 43 KB recording |
| Snap | JPEG `FFD8FF`, 9370 bytes |
| Transcode | FLV output, ffmpeg decode exit 0 |
| Cluster | `cluster_flv`; QUIC edge |
| gRPC | h2 `ListStreams` + `GetStatus` |

## Feature surface

- 26 official plugin crates present (10 protocol, 6 media, 5 utility,
  3 device, 2 room) + `m7s-codec`/`m7s-sdk`/`m7s-config`.
- `/api` + `/v6/api`, Web SDK/player, Admin stream delete/plugin config,
  JWT + file/MySQL user store, gRPC.
- Lock-free SPMC ring with Arc zero-copy; HTTP-FLV/QUIC/HLS/RTSP/WHEP
  consume it (WHEP via tap compatibility path).

## Known gaps (not yet V6 parity)

- HLS LL-HLS/fMP4/encryption.
- WebRTC Simulcast/SVC.
- GB28181 recording/playback/PTZ with real devices.
- ONVIF Media2/events.
- SQL-backed stream/room metadata (users only).
- Admin advanced configuration UI.
- WebTransport browser client.
- Some plugin crates are core primitives, not full V6 business logic
  (e.g. Live gifts/PK, V4L2/ALSA hardware paths).

## Verification commands

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release --workspace
node scripts/load_test.cjs 10000 10
```

## 2026-11-16 final re-test

```text
HTTP-FLV 10K: 10000/10000, 17.18s
A/V fMP4: FFmpeg exit 0 (trex full-box fix)
LL-HLS: PART + PRELOAD-HINT + blocking _HLS_msn/_HLS_part + part wait
WHIP: static + dynamic RID, NACK/RTT auto switch
```