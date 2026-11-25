# P4 SRT

## Listener

- UDP bind: `0.0.0.0:9000`
- SRT implementation: `sapflow-srt` (pure Rust, sans-IO)
- supported: HSv5 caller/listener, stream_id, live data

## Verified

FFmpeg SRT publish:

```powershell
ffmpeg -re -f lavfi -i testsrc=size=320x240:rate=25 -t 5 `
  -c:v libx264 -preset ultrafast -tune zerolatency -pix_fmt yuv420p `
  -f mpegts "srt://127.0.0.1:9000?streamid=live/test"
```

Server log:

```text
SRT listening on 0.0.0.0:9000
SRT connect 127.0.0.1:51308 stream_id="live/test"
SRT closed ConnId(...): Some(Shutdown), total=118252 bytes
```

## Not yet implemented

- MPEG-TS demux (PAT/PMT/PES -> H264/AAC);
- push SRT media into `StreamManager` / FLV;
- SRT passphrase encryption;
- WebTransport and ONVIF.

## SRT encryption

Set `M7S_SRT_PASSPHRASE` (10..79 bytes) before starting the server.

Verified:

```text
M7S_SRT_PASSPHRASE=secret12345
ffmpeg ... -f mpegts "srt://127.0.0.1:9000?streamid=live/enc&passphrase=secret12345"
API: key=live/enc, bytes=165587, cached_tags=251
```

## SRT egress（2026-11-21）

- `m7s_plugin_srt::push(stream, target, stream_id)`；
- 从 stream 读取 tag，经 `Segmenter` 生成 MPEG-TS；
- `sapflow_srt::Caller` 握手后按 1316B 分片发送，维持 buffered<8；
- HTTP 入口：
  ```text
  GET /api/srt/push?stream=live/src&url=srt://127.0.0.1:9000&id=live/out
  ```

测试：

```text
tests/srt_egress.rs::srt_egress_pushes_stream ... ok
```

## Not yet implemented

- SRT 双向/文件模式；
- WebTransport egress；
- ONVIF 真机。