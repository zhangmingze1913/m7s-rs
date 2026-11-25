# Protocol Compatibility Matrix (Monibuca V6 target)

Date: 2026-11-16

| Protocol | Ingest | Egress | Evidence | Status |
|---|---|---|---|---|
| RTMP | ✅ | ✅ | `rtmp_publish`, `http_flv` tests | ✅ |
| HTTP-FLV | — | ✅ | 10K viewers, `flv_magic=FLV` | ✅ |
| HLS | — | ✅ | `hls_http` test, `#EXTM3U` | ✅ (TS only) |
| RTSP | — | ✅ | `rtsp_describe_setup_play`, FFmpeg SETUP/PLAY | ✅ core; AAC demo frame issue |
| SRT | ✅ | — | FFmpeg SRT push, `bytes=165587` | ✅ |
| WebRTC WHEP | — | ✅ | Chrome `VIDEO 320x240 frames=206`, audio | ✅ |
| WebRTC WHIP | ✅ | — | `live/whip bytes=113270` | ✅ |
| WebTransport | ✅ | — | `live/wt bytes=81091` | ✅ datagram |
| GB28181 | ✅ | — | 12 SIP/PS/RTP tests | ✅ core |
| ONVIF | ✅ | — | Device/Media/Discovery, WS-Security | ✅ core |
| MP4 | — | ✅ | `live_mp4.mp4 ftyp=ftyp` | ✅ |
| Snap | — | ✅ | JPEG `FFD8FF` | ✅ |
| Transcode | ✅ | ✅ | FLV output, ffmpeg decode | ✅ |
| Cluster (HTTP/QUIC) | — | ✅ | `cluster_flv`, QUIC edge | ✅ |
| gRPC | ✅ | ✅ | h2 `ListStreams`/`GetStatus` | ✅ |
| HLS LL/fMP4 | — | ❌ | not implemented | ❌ |
| WebRTC TURN/SVC | — | ❌ | not implemented | ❌ |
| GB28181 录像/回放/PTZ 实机 | ❌ | ❌ | not implemented | ❌ |
| ONVIF Media2/事件 | ❌ | ❌ | not implemented | ❌ |

## Notes

- RTSP matrix run showed FFmpeg SETUP/PLAY; the demo's synthetic AAC produced
  a decode error. Re-test with a real FFmpeg AAC push:
  `ffmpeg -rtsp_transport tcp -i rtsp://... -t 2 -f null -` exited 0
  (only a non-monotonic DTS warning).
- HLS is MPEG-TS only; LL-HLS/fMP4/encryption remain.
- WebTransport uses unreliable datagrams; reliable stream mapping remains.

## Verdict

Core protocol surface is functional and repeatably tested. Full V6 parity is
not yet reached due to the gaps above.

## WebRTC boundary (2026-11-16)

| Capability | Status |
|---|---|
| Simulcast RID | ✅ |
| Manual RID switch | ✅ `/api/whip/rid` |
| Auto NACK/RTT switch | ✅ heuristic |
| REMB | ❌ |
| TWCC | ❌ |
| SVC | ❌ (encoder/decoder layer) |