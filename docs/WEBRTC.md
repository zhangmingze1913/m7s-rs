# P3 WebRTC / WHEP

## Endpoint

```text
POST /whep/<app>/<name>
port 8081
body: SDP offer
response: 201 + SDP answer
```

## Verified browser playback

Client: Chrome for Testing 157, headless, `ontrack` + video element.

Result:

```text
HTTP 201
TRACK video
ICE connected
PC connected
RTP video packets=1611 bytes=428864
VIDEO 320x240 frames=446
FINAL ice=connected pc=connected track=1
```

This proves:

- WHEP SDP offer/answer;
- ICE/DTLS/SRTP;
- H.264 RTP delivery;
- browser-side H.264 decode to 320x240;
- 446 decoded frames.

## Implementation notes

- Track must be added **before** `set_remote_description`; otherwise the
  packetizer is not bound and `write_sample` silently does no-op.
- H.264 `sdp_fmtp_line` should include `packetization-mode=1` and
  `sprop-parameter-sets`.
- Keyframes should carry SPS/PPS in Annex B before the IDR.
- The server sends the cached GOP only after ICE is connected.

## Re-run

```powershell
# terminal 1
.\target\debug\m7s-rs.exe --config config\server.toml

# terminal 2
.\node_modules\ffmpeg-static\ffmpeg.exe -re `
  -f lavfi -i testsrc=size=320x240:rate=25 -t 25 `
  -g 25 -keyint_min 25 -c:v libx264 -preset ultrafast -tune zerolatency `
  -pix_fmt yuv420p -f flv rtmp://127.0.0.1:1935/live/ff

# terminal 3
node tools\whep_check.js
```

## Not yet covered

- audio track;
- WHIP ingest;
- TURN / public-network ICE;
- non-loopback candidate selection.

## A/V playback (split sessions)

The current webrtc-rs path with one video transceiver and one audio transceiver
in the same PeerConnection still leaves the video track at 0 frames. A working
A/V path is to open two WHEP sessions:

- video-only offer: H.264 320x240;
- audio-only offer: PCMU (AAC is transcoded by ffmpeg to 8 kHz mono).

Chrome result:

```text
VIDEO HTTP 201
VIDEO TRACK
AUDIO HTTP 201
AUDIO TRACK
VIDEO 320x240 frames=175
AUDIO ready=4 time=8.938
FINAL v=connected a=connected
```

Test page: `tools/whep_av_split.html`.

The single-session A/V limitation remains open.
## WHIP ingest

`POST /whip/<app>/<name>` on port 8082 with an SDP offer.

- receives H.264 RTP;
- `rtp::codecs::h264::H264Packet` with `is_avc = true`;
- SPS/PPS -> FLV AVC sequence header;
- IDR/slice -> FLV video tags;
- pushes into the shared `StreamManager`.

Chrome canvas WHIP test:

```text
CODECS a=rtpmap:102 H264/90000
HTTP 201
PUSHED ice=connected
API streams: {"key":"live/whip","bytes":101055,"cached_tags":111}
```

FFmpeg pulls the generated FLV:

```text
ffmpeg -i http://127.0.0.1:8180/flv/live/whip.flv -t 2 -f null -
exit 0
```

Test page: `tools/whip_test.html`; runner: `tools/whip_check.js`.
## Single-session A/V (fixed)

WHEP now sets the offered transceivers' senders after
`set_remote_description`, instead of adding new transceivers. H264 codec
preference is pinned to payload 102.

Chrome test with mDNS local-IP hiding disabled:

```text
OFFER candidates=2
HTTP 201
TRACK video
TRACK audio
ICE connected
PC connected
RTP video packets=1164 bytes=239655
VIDEO 320x240 frames=147
AUDIO ready=4 time=7.93
FINAL ice=connected pc=connected track=2
```

This closes the earlier "single PeerConnection A/V has video 0" limitation.

Chrome flag used by `tools/whep_single_check.js`:

```text
--disable-features=WebRtcHideLocalIpsWithMdns
```
## TURN/ICE servers

WHEP/WHIP read:

```text
M7S_TURN_URL   comma-separated turn:/turns: URLs
M7S_TURN_USER
M7S_TURN_PASS
```

and build `RTCConfiguration.ice_servers`. Test: `ice_tests::parses_turn`.
Simulcast/SVC not implemented.
## Simulcast RID selection (WHIP)

- Parse `a=simulcast:send h;m;l` from WHIP offer.
- Accept only the preferred RID (first, e.g. `h`); ignore other RIDs.
- Offers without simulcast are unchanged.

Test: `simulcast_tests::picks_first_rid`.
SVC itself is not implemented; this is RID simulcast selection only.
## Dynamic RID switching

- `GET /api/whip/rid?stream=app/name&rid=l` sets a per-stream RID override.
- WHIP reads all video tracks but forwards packets only from the effective RID.
- If no override, the SDP simulcast preference is used.

Tests: `rid_override_tests::override_map`, `simulcast_tests::picks_first_rid`.
## Automatic RID switch (NACK heuristic)

Every 2s WHIP reads `get_stats`, sums `nack_count` for inbound video:

- delta > 10 -> RID `l`
- delta == 0 -> RID `h`
- otherwise keep current

This is a coarse congestion heuristic, not full bandwidth estimation.
Tests: `auto_rid` cases in `rid_override_tests`.
## Auto RID: NACK + RTT

Heuristic updated:

- `nacks > 10` or `rtt > 300ms` -> RID `l`
- `nacks == 0` and `rtt < 150ms` -> RID `h`
- otherwise keep current

Tests cover low/high RTT cases.
## Bandwidth estimation / SVC boundary

Exposed at `GET /api/webrtc/capabilities`:

```json
{"simulcast_rid":true,"auto_layer_switch":true,"remb":false,"twcc":false,"svc":false}
```

- Automatic layer switching uses **NACK delta + RTT**, not REMB/TWCC.
- The `webrtc` 0.11 crate does not expose REMB/TWCC estimates to the app;
  implementing them would require a custom RTCP intercept path.
- SVC requires an SVC-capable encoder and decoder chain. The current
  H.264/VP8 path does not provide spatial layers, so SVC is explicitly
  unsupported rather than stubbed.
- Simulcast RID and manual/dynamic layer selection are supported.
## REMB parser

`parse_remb(packet)` decodes RTCP PSFB FMT=15 REMB exp/mantissa to bps.
Test: `remb_tests::parses_remb`.

The WHIP path still cannot intercept RTCP, so REMB is **not wired** to layer
selection; auto switch remains NACK+RTT.
## REMB wiring via custom Interceptor

No webrtc fork was needed. `RembInterceptor` implements
`interceptor::Interceptor`, wraps `RTCPReader`, downcasts
`ReceiverEstimatedMaximumBitrate`, and updates the stream RID:

```text
bitrate < 300 kbps  -> l
bitrate > 1 Mbps    -> h
otherwise           -> m
```

WHIP builds the API with `with_interceptor_registry(crate::registry(&key))`.
Test: registry builds and REMB parse test pass.

TWCC is still not decoded; SVC still unsupported.
## TWCC decode wiring

`RembReader` also downcasts `TransportLayerCc` chunks:

- StatusVectorChunk symbols counted;
- `PacketNotReceived` ratio > 5% -> RID `l`;
- zero loss -> RID `h`;
- else `m`.

This is loss-based TWCC feedback decoding, not a full TWCC bandwidth estimator.
Tests: `twcc_rid` cases in `rid_override_tests`.
## TWCC coarse bandwidth estimator

`TwccEstimator` tracks reference_time and loss-adjusted packet count,
assumes 1200-byte average packets, and returns bps between feedbacks.
Wired into `RembReader`: estimated bps selects RID via `rid_for_bps`
(<300k -> l, >1M -> h, else m).

Tests: `twcc_est_tests::estimates_bps`, `twcc_rid`.
Not a full TWCC send-history estimator; packet size is an average.
## TWCC send history + arrival time (2026-11-17)

- `TwccEstimator::record_sent(seq, bytes)`;
- `estimate(recv_seqs, elapsed_us)` uses sent sizes (fallback 1200B);
- `on_feedback` reconstructs received sequences from RunLength/StatusVector chunks,
  sums `recv_deltas` as arrival span (fallback 64ms reference delta), returns bps;
- RID selection still via `rid_for_bps`.

Tests: `twcc_est_tests::{estimates_with_send_history,empty_feedback_is_none,missing_history_uses_1200}`.

## SVC explicit unsupported (2026-11-17)

`/api/webrtc/capabilities`:

```json
{"svc":false,"svc_reason":"no SVC encoder in current H.264 stack"}
```

No SVC encoder/decoder is integrated; no stub.
## WHEP SDP E2E (2026-11-18)

`tests/whep_sdp.rs`：

- webrtc 客户端生成真实 offer（recvonly video）；
- POST `/whep/live/whep`；
- 断言 201 且 answer 含 `v=0`、`m=video`。

```text
whep_returns_sdp_answer ... ok
```
## WHIP SDP E2E (2026-11-18)

`tests/whip_sdp.rs`：

- webrtc 客户端创建 sendonly video transceiver；
- POST `/whip/live/whip`；
- 断言 201 且 answer 含 `m=video`。

```text
whip_returns_sdp_answer ... ok
```
## WHEP audio negotiation (2026-11-18)

`tests/whep_sdp.rs` 增加音频：

- 推 AAC sequence header（type 8）；
- 客户端添加 recvonly audio transceiver；
- answer 断言 `m=audio` 与 `PCMU/8000`。

```text
whep_returns_sdp_answer ... ok
```
## WebRTC ICE loopback 验证（2026-11-18）

修正对照测试的 gathering 顺序后：

```text
webrtc_loopback::loopback_ice_connects ... ok
```

说明本机 webrtc 0.11 ICE/UDP 可用；之前 "环境阻断" 结论**不成立**。
WHEP 媒体面问题被隔离为 WHEP 自身：服务端日志显示 remote ufrag/pwd 已收到，
但 `pingAllCandidates ... no candidate pairs`，即 offer 的 candidate 未被加入服务端 agent。

## WHEP 媒体面通过（2026-11-21）

根因不是 WHEP 服务端，而是测试客户端设置了 `set_nat_1to1_ips`，
候选地址被改写。移除客户端 NAT 改写后：

```text
tests/whep_media.rs  ICE Connected + RTP received ... ok
tests/whip_media.rs  上行 RTP -> FLV tag ... ok
```

## TWCC per-packet 估算（2026-11-21）

取代上面的“粗估/平均包长”实现：

- `TwccEstimator::record_sent_at(seq, bytes, sent_us)` 记录每包大小与发送时间；
- RTP writer interceptor 在 WHEP/WHIP 发送路径自动调用；
- `on_feedback(TransportLayerCc)`：
  - 解析 RunLength/StatusVector，得到 received/failed 序列；
  - 用 `reference_time + recv_deltas` 重建每包到达时间；
  - `estimate_arrivals` 用真实字节/到达跨度计算 bps；
  - 发送跨度与到达跨度差 > 20ms 时按队列延迟下调（2/3）。
- WHEP/WHIP MediaEngine 注册 `transport-cc` header extension；
  WHEP SDP 测试断言 answer 含 `transport-cc`。
- `/api/webrtc/capabilities`：`remb=true, twcc=true, twcc_per_packet=true, transport_cc=true`。

测试：

```text
twcc_est_tests::{per_packet_arrival_and_send_times, queue_delay_reduces_estimate, feedback_parses_arrival_deltas}
whep_media_plane ... ok
whip_media_plane ... ok
```