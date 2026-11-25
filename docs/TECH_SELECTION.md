# 技术选型方案 — m7s-rs

## 1. Language / runtime

| Choice | Alternatives | Decision |
|---|---|---|
| Rust 2024, MSVC on Windows | C++/Go | Memory safety, zero-GC hot path, existing codebase |
| Tokio 1.x multi-thread | async-std/glommio | Ecosystem, quinn/webrtc compatibility |
| `ArcSwap` + atomics | Mutex/RwLock/crossbeam | Lock-free SPMC ring, 117 ns/write pooled |

## 2. Media/protocol

| Area | Crate | Reason |
|---|---|---|
| FLV/AVC/AAC/TS | in-house `m7s-codec` | avoid FFmpeg for hot path |
| RTSP/SDP/RTCP | in-house `m7s-plugin-rtsp` | full control, testable helpers |
| SRT | `sapflow-srt` | Rust SRT, live mode |
| WebRTC | `webrtc` 0.11 | pure Rust, H264/PCMU |
| QUIC/WebTransport | `quinn` 0.11 + `web-transport-quinn` | same QUIC stack as cluster |
| HTTP/2 gRPC | `h2` + `prost` | no protoc/system deps |
| TLS | `rustls` + `ring` | no OpenSSL on Windows |
| HLS fMP4 | in-house box writer | no mp4 crate for fragmented output |

## 3. Data layer

| Need | Choice | Reason |
|---|---|---|
| Users file | JSON + SHA-256 | zero deps, simple |
| MySQL | `mysql_async` (rustls) | async, matches Tokio |
| Config | TOML + serde | human-editable |
| Cluster metadata | HTTP/QUIC JSON | no external DB required |

## 4. API/security

| Need | Choice |
|---|---|
| REST | hand-rolled HTTP/1.1 on Tokio |
| Auth | static token, JWT HS256, MySQL/file users |
| Web SDK | vanilla JS WHEP/HLS/flv |
| Admin | server-rendered HTML + fetch |

## 5. Rejected options

| Option | Why rejected |
|---|---|
| FFmpeg for all ingest | heavy, non-deterministic latency |
| Go/C++ | rewrite cost, memory safety |
| OpenSSL | Windows build friction |
| gRPC via protoc | no protoc guaranteed; hand-written h2/prost |
| External DB for cluster | operational overhead for single binary |
| LL-HLS via full TS byte ranges | non-compliant; needs CMAF parts |

## 6. Performance targets and evidence

| Target | Evidence |
|---|---|
| ~100 ns ring write | 117.2 ns/write pooled Arc |
| 10K HTTP-FLV viewers | 10000/10000, 15.12 s |
| Low-copy egress | Arc-shared FlvTag through ring |
| Browser WHEP A/V | Chrome 320x240, 206 frames |
| WHIP ingest | 113 KB in stream |

## 7. Portability

Current CI/dev is Windows MSVC. Tokio/quinn/rustls/webrtc are cross-platform;
Linux CI is a follow-up gate. No platform-specific syscalls in media paths.
