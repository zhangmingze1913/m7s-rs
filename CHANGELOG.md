# Changelog

## Unreleased (v0.1.0, 2026-11)

### Added

- 31-crate Rust workspace: RTMP, HTTP-FLV, HLS/LL-HLS/fMP4, RTSP, SRT,
  WebTransport, WHEP/WHIP, QUIC cluster, ONVIF, GB28181, gRPC, Room/Live/
  Meeting, Admin RBAC/audit, WASM, SEI/mix/transcode.
- HTTP-FLV fan-out via a shared `tokio::sync::broadcast` ring.
- Performance gate: `scripts/perf_gate.ps1`.
- Delivery handbook: `docs/DELIVERY.md`.
- Web SDK ABR for WebRTC/hls.js/FLV.
- WASM Ed25519 signature/whitelist/fuel.
- License activation/revocation service with persisted revocation and limits.

### Performance

- 5K HTTP-FLV: `ok=5000/5000`, ~54% CPU, p99 ~25ms, ~10MB RSS.
- 10K HTTP-FLV: `ok=10000/10000`, ~45% CPU, p99 ~255ms, ~14MB RSS.
- 20K: 16.3K/20K with two Node clients; server CPU ~61%, client/OS limited.

### Known limitations

- 100K requires multi-host validation.
- SVC requires an external encoder; only NAL parsing/layer filtering is present.
- ONVIF/GB28181/SRT/WebRTC hardware acceptance is not complete.
- Native H.264/AAC encoding is still delegated to FFmpeg.
- Production billing/multi-tenant licensing is not implemented.
