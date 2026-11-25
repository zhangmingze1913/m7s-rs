# 重新规划：从当前原型到 V6 对标

## 0. 规划原则

1. **先内核，后协议数量**：没有无锁/零拷贝，不做 26 插件；
2. **先可验证，后宣称**：每个阶段必须有外部客户端/压测证据；
3. **先单机性能，后集群**：单机 10K 未过，不谈 QUIC 集群；
4. **控制面与媒体面分离**：OntoDB 等只放控制面。

## 1. 阶段

### P0：冻结与基准（1–2 周）

- 固定 RTSP/HLS/GB28181 当前行为；
- 建 CI：fmt、clippy、单测、集成、release；
- 统一压测：1K/5K/10K、CPU/内存/P99、72h；
- 固定 VLC/FFmpeg 客户端环境；
- 输出 baseline 报告。

验收：可重复的基准脚本与报告；RTSP VLC 闭环。

### P1：数据面重构（4–6 周）——最高优先级

- `MediaFrame`/`Track` 统一模型；
- lock-free SPMC RingBuffer；
- `ArcSwap` 订阅者列表、`Bytes` 零拷贝；
- 对象池、bounded 背压、关键帧请求；
- 单 crate → workspace：`codec`、`core`、`media`、`protocol`、`service`、`app`。

验收：单流 10K 订阅；P99 延迟报告；无数据竞争（Loom/Miri）；与当前实现对比。

### P2：RTSP / GB28181 完整（6–8 周）

RTSP：Digest、UDP/TCP、RTCP SR/RR 周期、VLC/ffmpeg、重连、AAC。
GB28181：Digest、Catalog、INVITE/BYE、SDP、RTP/PS、设备联调。

验收：VLC + 真实 GB28181 设备/平台。

### P3：WebRTC（8–10 周）

- ICE/DTLS/SRTP；
- WHIP/WHEP；
- NACK/PLI/FIR、拥塞控制；
- 浏览器互通。

验收：Chrome/Firefox/Safari；弱网 20% 丢包。

### P4：SRT / WebTransport / ONVIF（6–8 周）

- SRT 握手/ARQ/加密；
- WebTransport HTTP/3；
- ONVIF 发现/能力/Profile。

### P5：QUIC 集群（6 周）

- Quinn Relay；
- 心跳 Healthy/Suspect/Offline；
- 会话目录、分配、重定向；
- Origin 故障切换、级联 Edge。

验收：1 Origin + 3 Edge；Origin kill 5s 切换；混沌测试。

### P6：26 插件 + 动态加载（6–8 周）

- codec/sdk/config-framework；
- 26 插件 crate；
- 动态库/WASM 沙箱、版本/ABI；
- MP4/Snap/SEI/Transcode 等媒体插件。

验收：第三方插件不改主程序加载；故障隔离。

### P7：商业化（持续）

- Admin/Web SDK/监控；
- TLS/mTLS、租户、审计、限流；
- K8s/Helm、灰度/回滚、SLA；
- 安全评估/压测/灾备。

## 2. 优先级调整

原计划“先 RTSP/GB28181/WebRTC 再内核”不可取：

- 当前 10K 未过、无 P99、无零拷贝；
- 继续堆协议只会把问题放大。

新顺序：**P0 基准 → P1 数据面 → P2 协议完整 → P3 WebRTC → P4 其余 → P5 集群 → P6 插件 → P7 商业化**。

## 3. 团队与周期

- 8–12 人，12–18 个月；
- P0/P1 是分水岭：P1 不通过，不进入 P3/P5；
- 每个阶段必须有外部客户端/压测/故障注入证据。

## 4. 明确不做

- 不用 26 个空壳插件充数；
- 不在 10K 未过时宣称“万路”；
- 不在 VLC 未过时宣称 RTSP 完成；
- 不在单机未稳时宣称 QUIC 集群。
