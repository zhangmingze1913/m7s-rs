# P4 WebTransport

## Server

- UDP `0.0.0.0:4443`;
- QUIC + HTTP/3 via `web-transport-quinn` 0.13.6;
- TLS certificate: `tests/fixtures/server.pem`, key `tests/fixtures/server-key.pem`
  (override with `M7S_WEBTRANSPORT_CERT` / `M7S_WEBTRANSPORT_KEY`);
- WebTransport CONNECT accepted;
- server echoes datagrams.

## Verified

Rust client (`examples/wt_echo.rs`, URL `https://127.0.0.1:4443`):

```text
ECHO hello-webtransport
```

This proves:

- QUIC handshake;
- HTTP/3 WebTransport CONNECT;
- WebTransport datagram send/receive;
- server echo data plane.

Windows firewall inbound UDP 4443 was required for the test.

## Not yet implemented

- WebTransport media mapping (MPEG-TS/H264 over datagrams or streams);
- WebTransport browser client;
- certificate automation.

## Media ingest

Clients send MPEG-TS bytes as WebTransport datagrams to
`https://<host>:4443/live/<name>`.

Server:

- reuses `srt::TsDemux`;
- H.264/AAC -> FLV tags;
- pushes to `live/<name>`.

Verified with `examples/wt_ts.rs`:

```text
SENT 116372 bytes
API: key=live/wt, bytes=81091, cached_tags=125
ffmpeg -i http://127.0.0.1:8180/flv/live/wt.flv -t 1 -f null -
exit 0
```

Datagrams are unreliable; some TS packets may drop. Reliable media should use
WebTransport streams in a later iteration.
## WebTransport reliable stream ingest

Server now accepts bidirectional/uni streams in addition to datagrams:

```rust
session.accept_bi() -> recv.read(&mut buf) -> TsDemux
```

Tested with `examples/wt_stream.rs` (116372 TS bytes over one stream):

```text
SENT_STREAM 116372
live/wtstream bytes=81091 cached_tags=125
```

Datagram path unchanged.

## WebTransport egress（2026-11-21）

- `m7s_plugin_webtransport::push(stream, target_url)`：
  - 以 WebTransport client 连接远端；
  - 从 stream 读取 tag，经 HLS Segmenter 生成 MPEG-TS；
  - 按 1200B 分片通过 `send_datagram_wait` 发送。
- HTTP 入口：
  ```text
  GET /api/webtransport/push?stream=live/src&url=https://host:4443/live/out
  ```

测试：

```text
tests/webtransport.rs::webtransport_egress_pushes_stream ... ok
```

现在 WebTransport 覆盖：

```text
✅ datagram echo
✅ bi-stream TS ingest
✅ egress push（datagram 媒体）
```

仍未完成：浏览器端 SDK、可靠媒体流映射、certificate automation。