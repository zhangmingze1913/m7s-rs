# RTMP Server

实现：`crates/m7s-plugin-rtmp/src/server.rs`（根 crate 仅适配器）。

- publish：`connect/createStream/publish` → `push_tag`；
- play：`snapshot + tag_reader`；
- TLS：`tokio-rustls`（ring/tls12）；
- 录制：`RecordHook`；
- 鉴权：`RtmpConfig.publish_key`。

测试：

```text
tests/rtmp_publish.rs
tests/rtmp_play.rs
tests/rtmps.rs
```
