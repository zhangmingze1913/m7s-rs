# Monibuca V6 插件矩阵 vs m7s-rs

> Monibuca V6 的 “26 个插件 crate” 不等于 “26 种协议”。其中协议约 8 类，其余是媒体处理、工具、设备、房间插件。不能用 26 个空壳插件冒充 26 种协议。

## Monibuca V6 26 个插件 crate（按公开架构文档）

协议：RTMP、RTSP、FLV、HLS、WebRTC、SRT、WebTransport、GB28181、ONVIF  
媒体：MP4、Snap、SEI、Mix、Transcode、Crypto  
工具：Debug、LogRotate、Report、Crontab、Test、Cluster  
设备：HomeKit、V4L2、ALSA  
房间扩展：Live、Meeting

## m7s-rs 当前状态

| 插件/协议 | 状态 | 说明 |
|---|---|---|
| RTMP | ✅ 基础 | 推流/播放、AMF0、chunk |
| HTTP-FLV | ✅ | GOP 缓存 + 直播 |
| HLS | ⚠️ | H.264/AAC TS，FFmpeg 可解码，仍有时间戳/边界告警 |
| RTSP | ✅/⚠️ | TCP/UDP H.264+AAC 已与 FFmpeg 互通；VLC 能建连，播放未闭环 |
| WebRTC | ❌ | 无 ICE/DTLS/SRTP/WHIP/WHEP |
| SRT | ❌ | 无握手/ARQ/加密 |
| WebTransport | ❌ | 无 HTTP/3/QUIC |
| GB28181 | ⚠️ | SIP REGISTER（Digest）+ Catalog + MESSAGE；无 INVITE/RTP/PS |
| ONVIF | ❌ | 无设备发现 |
| MP4 录制 | ❌ | 仅 FLV 录制 |
| Snap/SEI | ❌ | 未实现 |
| Mix/Transcode | ❌ | 未实现 |
| Crypto | ⚠️ | 仅 TLS/HTTPS/RTMPS |
| Debug/LogRotate/Report | ❌ | 仅基础日志 |
| Crontab | ❌ | 未实现 |
| Test | ⚠️ | 13 单元 + 7 集成 |
| Cluster | ⚠️ | HTTP-FLV Relay，非 QUIC |
| HomeKit/V4L2/ALSA | ❌ | 未实现 |
| Live/Meeting | ❌ | 未实现 |

## 结论

- 当前新增的是 **RTSP 真实协议插件**，不是空壳；
- 26 个插件全量需要按协议族逐项实现、互操作和压测；
- 下一步应做 RTSP 与 FFmpeg/VLC 的完整互操作，再进入 GB28181/WebRTC，而不是给 26 个文件写 `todo!()`。
