# PDM — Product Data Management / 产品数据管理

Project: m7s-rs — Monibuca V6 compatible Rust streaming server
Date: 2026-11-16

## 1. Product structure

```text
m7s-rs
├─ core
│  ├─ m7s-codec      FLV/AVC/AAC/TS primitives
│  ├─ m7s-sdk        Plugin/Stream/Engine traits
│  ├─ m7s-config     TOML config
│  └─ engine         stream manager, ring, HTTP/gRPC, auth, HLS, cluster wrapper
├─ protocol plugins  flv rtmp rtsp srt webtransport gb28181 onvif webrtc hls
├─ media plugins     mp4 snap sei mix transcode crypto
├─ utility plugins   debug logrotate report crontab test
├─ device plugins    homekit v4l2 alsa
├─ room plugins      live meeting
├─ web               sdk.js / player.html / admin
├─ docs              design, matrix, acceptance
└─ tests             integration + protocol + load
```

## 2. WBS and status

| WBS | Item | Status |
|---|---|---|
| 1 | Lock-free SPMC RingBuffer (ArcSwap + seq) | Done, 117.2 ns/write pooled |
| 2 | 26 official plugin crates | Done, registry count 26 |
| 3 | FLV/RTMP/HTTP-FLV | Done, 10K viewers verified |
| 4 | HLS TS | Done |
| 5 | HLS fMP4 | Video init/moof/mdat + endpoint, FFmpeg decode OK |
| 5.1 | fMP4 audio | Not done |
| 5.2 | LL-HLS parts/reload | Parts + `_HLS_msn=N` wait, partial verification |
| 6 | RTSP server plugin | Done, TCP/UDP, FFmpeg A/V |
| 7 | SRT / WebTransport | Datagram + reliable stream |
| 8 | GB28181 SIP/Catalog/INVITE/PS/RTP | Signaling + playback sender; seek/speed; control partial |
| 9 | ONVIF Device/Media/Media2/Events | Basic done |
| 10 | WHEP/WHIP | Chrome A/V + WHIP push verified |
| 11 | Cluster HTTP/QUIC | Done, `cluster_flv` |
| 12 | Admin/Web SDK/JWT/MySQL/gRPC | Done at listed scope |
| 13 | Simulcast/SVC | Not done |
| 14 | Performance | 10K HTTP-FLV, 117 ns ring write |

## 3. Milestones

- M1 Media core + FLV/RTMP: complete.
- M2 Full protocol surface: complete for core paths.
- M3 Plugin isolation: complete (engine protocol modules re-export plugins).
- M4 Admin/API/security: complete at current scope.
- M5 V6 depth: HLS fMP4 audio/LL, Simulcast/SVC, GB playback controls.
- M6 Final acceptance: re-run matrix, 10K, latency, memory.

## 4. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| fMP4 audio box layout errors | No A/V playback | Build per-track moov, ffmpeg validation |
| LL-HLS part semantics | Browser stalls | PART sequence + blocking reload tests |
| Simulcast/SVC encoder dependency | Can't claim SVC | RID simulcast first, SVC only if encoder supports |
| GB playback file formats | Device incompatibility | FLV now; MP4 demux next |
| Windows-only validation | Portability | CI matrix Linux later |
| Cluster split-brain | Duplicate relays | Heartbeats + origin quorum |

## 5. Acceptance gates

1. `cargo clippy --workspace --all-targets -- -D warnings`.
2. `cargo test --workspace` all green.
3. `cargo build --release --workspace`.
4. Protocol matrix row has reproducible evidence.
5. Performance row re-run on release binary.

## 6. Change management

Every change must include: code, test, doc update, clippy/test/release run.
No protocol stub is marked complete.
