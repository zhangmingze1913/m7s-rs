# RTSP Pull（客户端，2026-11-22）

`src/rtsp_pull.rs`：

```rust
rtsp_pull::pull("rtsp://cam:554/live/main", streams, "live", "camera").await?;
```

自动重连：`pull()` 无限重试；`pull_with_retry(..., max)` 可限制次数，退避 200ms×2^n。

HTTP：

```text
GET /api/rtsp/pull?stream=live/camera&url=rtsp://cam:554/live/main
```

流程：

```text
TCP connect
→ DESCRIBE（解析 sprop-parameter-sets）
→ SETUP trackID=0 interleaved=0-1
→ PLAY
→ 读 $ 交织帧
→ H.264 FU-A / single NAL 重组
→ Annex B → AVCC
→ AAC MPEG4-GENERIC AU 头解析
→ 写入 FLV video/audio tags + 配置头
```

测试：

```text
tests/rtsp_pull.rs::rtsp_pull_video ... ok
tests/rtsp_pull_reconnect.rs::rtsp_pull_reconnects ... ok
tests/rtsp_pull_rtcp.rs::pull_sends_rtcp_receiver_report ... ok
```

覆盖：

- 本机 RTSP server 作为源；
- 拉流写入 `live/pulled`；
- 断言目的流出现 AVC sequence header 与视频 tag。

可与 ONVIF 组合：\n\n```text\nGET /api/onvif/pull?endpoint=http://cam/onvif/device_service&profile=profile1&stream=live/cam\n```\n\n## 限制

- 视频 H.264 + 音频 AAC；自动重连；收到 SR 后发送 RTCP RR；
- 只支持 TCP interleaved；
- 未验证厂商摄像头差异与弱网；
- 未接入 ONVIF `GetStreamUri` 自动拉流（可由调用方拼接）。

## RTMP Play��2026-11-22��

��� server �� play ʹ�� `snapshot + tag_reader`�����ԣ�

```text
tests/rtmp_play.rs::rtmp_play_returns_cached_tag ... ok
```

## SVC 层过滤（2026-11-22）

M7S_SVC_MAX_DEPENDENCY=0 时，RTSP pull 保留 base + 指定 dependency 层；
NAL 14/20 现已作为单 NAL 接受。
测试：tsp_pull::svc_tests::svc_depacketize_and_filter。
