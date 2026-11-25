# SVC 外部编码器方案

Date: 2026-11-16

## 约束

当前 WHIP 只处理 H.264 单层 RTP；`webrtc 0.11` 没有 SVC 层提取/转码。
客户端 SVC（VP9/AV1）也无法在服务端直接拆层。

## 可选方案

### A. 客户端 simulcast（已支持）

- 客户端发多 RID，服务端选层；
- 不是 SVC，但覆盖大多数弱网场景。

### B. 外部 SVC 编码器 + 服务端转发

- FFmpeg/OpenH264 产生多空间层；
- 每层单独 RTP 流；
- 服务端按 RID 转发；
- 需要编码器支持 `svc` 模式（x264 不支持 SVC；需要 JSVM/OpenH264/SVT？）。

### C. 服务端转码分层

- 收单层 → 解码 → 多分辨率编码（simulcast 模拟）；
- CPU 成本高，延迟增加；
- 可用现有 transcode 插件扩展。

### D. 明确不支持

- `/api/webrtc/capabilities` 返回 `svc:false`；
- 文档说明，不写桩。

## 推荐路线

1. 保持 RID simulcast；
2. 对 SVC 客户端，明确返回不支持；
3. 如需 SVC，采用 **C（服务端转码分层）**，复用 transcode FFmpeg 管线，
   产出的多层再走 WHIP/WHEP；
4. 不以“SVC”名义宣称支持，直到有真实层切换验证。

## 验收

- 外部编码器产生 2 层；
- 服务端按层转发；
- 客户端切换层无黑屏；
- 自动化测试覆盖层选择。

当前状态：未实现，`svc:false`。
