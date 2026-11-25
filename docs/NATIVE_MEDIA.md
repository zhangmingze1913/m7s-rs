# 原生媒体核心 / 告警（2026-11-21）

## SEI（`m7s-plugin-sei`）

```rust
build_sei(uuid: [u8;16], payload: &[u8]) -> Vec<u8>
parse_sei(nal: &[u8]) -> Option<([u8;16], Vec<u8>)>
insert_sei(annexb: &[u8], uuid, payload) -> Vec<u8>
```

- 生成 Annex B `user_data_unregistered` SEI NAL；
- 支持 255 字节分段长度；
- 解析 UUID + payload；
- 可插入 access unit 前。

测试：

```text
tests::extracts_sei
tests::build_and_parse_roundtrip
```

## Mix（`m7s-plugin-mix`）

新增纯 Rust 合成原语：

```rust
blend_region(base, base_w, base_h, top, top_w, top_h, x, y, alpha)
stack_horizontal(left, right, w, h)
stack_vertical(top, bottom, w, h)
```

测试：

```text
native_tests::region_and_stack
```

FFmpeg 的 `mix`/`mix_bytes` 仍保留用于编码输出。

## Transcode（`m7s-plugin-transcode`）

新增：

```rust
scale_rgb(src, sw, sh, dw, dh) -> Vec<u8>
```

最近邻 RGB 缩放，作为原生合成/缩放原语；H.264 编码仍由 FFmpeg。

测试：

```text
native_scale_tests::scales_nearest
```

## SEI HTTP 注入（2026-11-22）

```text
GET /api/sei?stream=live/x&text=hello
``

构建 `user_data_unregistered` SEI，转为 AVCC 后作为视频 tag 注入。
测试：`tests/sei_http.rs`。

## NAL 工具（`m7s-codec`）

```rust
split_annexb(data) -> Vec<&[u8]>
annexb_to_avcc(annexb) -> (keyframe, Vec<u8>)
avcc_to_annexb(avcc) -> Vec<u8>
``

测试：`nal_tests::annexb_avcc_roundtrip`。

SVC NAL：

```rust
is_svc(nal) -> bool
parse_svc_header(nal) -> Option<SvcHeader>
``

`svc_select_layers(annexb, max_dependency)` 保留 base + 指定 dependency；
SVC 编码/解码仍需外部编码器。
测试：`svc_tests::parses_svc_extension`。

媒体信息 API：

```text
GET /api/media/info?stream=live/x
``

返回 AVC 宽高/profile/level/fps、AAC object/sample_rate/channels。
测试：`tests/media_info.rs`。

SPS 尺寸：

```rust
sps_dimensions(sps) -> Option<(u32, u32)>
``

测试用 ffmpeg 生成 320x240 H.264 并解析 SPS。

AVC 配置记录：

```rust
build_avc_config(sps, pps) -> Vec<u8>
parse_avc_config(record) -> Option<(sps, pps)>
``

测试：`avc_config_tests::avc_config_roundtrip`。

AAC AudioSpecificConfig：

```rust
parse_asc(asc) -> Option<AudioSpecificConfig>
build_asc(cfg) -> Option<[u8;2]>
``

测试：`asc_tests::asc_roundtrip`。

AAC ADTS：

```rust
adts_to_raw(frame) -> Option<(Vec<u8>, [u8;2])>
raw_to_adts(raw, asc) -> Vec<u8>
``

测试：`adts_tests::adts_roundtrip`。

## 流生命周期事件

`src/events.rs` 广播 `stream.created` / `stream.removed`；
SSE：`GET /api/events/stream?count=N`。
测试：`tests/events.rs`。

## 连接指标（2026-11-23）

`/metrics`：`m7s_active_connections`、`m7s_max_connections`；
`/api/connections` 返回 active/limit。

## 告警（`src/alerts.rs`）

`GET /api/alerts`：

```json
{"code":0,"alerts":[{"code":"streams.high","severity":"warning",...}]}
```

阈值：

```text
M7S_ALERT_STREAMS  (default 9000)
M7S_ALERT_DROPPED  (default 1000)
```

当前规则：

- `streams.high`；
- `dispatch.dropped`。

测试：

```text
alerts::tests::thresholds
tests/alerts_http.rs::alerts_endpoint
```

SSE 告警：`GET /api/alerts/stream` 默认长连接，每 500ms 一个事件；`?count=3` 用于测试。断线重连由客户端负责。

仍开放：pprof/tracing、RSS/CPU 阈值。
