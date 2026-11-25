# Monibuca v6 对标与商业化路线图

## 0. 结论先行

“完全实现 Monibuca v6 并直接商业化替换”不是一次代码生成能完成的任务。按当前公开能力对标，至少包含：

- 8 类协议栈（RTMP、HTTP-FLV、HLS、RTSP、WebRTC、SRT、GB28181、WebTransport）
- QUIC 集群、节点发现、心跳三态、故障切换、负载分配
- 插件动态加载/ABI/沙箱
- 无锁 RingBuffer、对象池、背压
- 录制/截图/转码/MP4 封装
- Admin、Web SDK、监控、鉴权、TLS
- 性能、稳定性、安全与商业支持体系

合理投入约为 **8–12 人核心团队、12–18 个月**；其中 WebRTC、GB28181、SRT、WebTransport 每一项都需要独立协议负责人和互操作测试环境。单次会话只能交付可验证的阶段性增量。

当前 `m7s-rs` 是 **Phase 0 核心子集**，不是完整替代品。

## 1. 阶段划分

### Phase 0（已完成，当前仓库）

- RTMP 推流/播放基础链路
- HTTP-FLV
- GOP 缓存 + broadcast
- Origin/Edge HTTP 控制面 + HTTP-FLV Relay
- HTTP API
- 6 单元 + 2 集成测试
- 明确“不完整”的边界

验收：RTMP 推流可在 HTTP-FLV 播放；Edge 可回源。

### Phase 1：HLS + 录制 + Admin + 运维基础（2–3 个月）

- [x] FLV → TS（H.264/AAC）封装
- [x] HLS 切片、m3u8、GOP 对齐
- [x] FLV 录制、磁盘目录
- [x] Admin 静态页 + 流列表
- [x] API Token
- [x] 基础 `/metrics` 指标
- [x] 1000/5000 并发 HTTP-FLV 实测（见 LOAD_TEST.md）
- [ ] 10000 并发（当前约 57%，未通过）
- [x] 内置 HTTPS（rustls，证书文件）
- [x] RTMPS
- [x] mTLS 端到端（客户端证书/无证书拒绝）
- [ ] 证书热加载
- [ ] Prometheus 完整指标/日志/追踪
- [x] FFmpeg 可识别 HLS 的 H.264/AAC 并解码（仍有时间戳/边界告警）
- [ ] VLC/Safari 互操作
- [ ] 低延迟 HLS/LL-HLS
- 验收：ffplay/VLC/Safari 播放 HLS；录制可回放；Admin 可操作；压测 1k 流。

### Phase 2：RTSP + GB28181（3–4 个月）

- RTSP 服务端/客户端：OPTIONS/DESCRIBE/SETUP/PLAY/TEARDOWN、RTP/RTCP、H.264/H.265/AAC
- GB28181：SIP 注册/心跳/目录/Invite/Bye、RTP/PS、TCP/UDP 被动/主动
- 验收：FFmpeg/VLC RTSP 互通；GB28181 模拟平台/设备联调；7×24 稳定性。

### Phase 3：WebRTC（3–4 个月）

- ICE（STUN/TURN）、DTLS、SRTP、RTCP
- WHIP/WHEP、SDP 协商、Simulcast、NACK/PLI/FIR、拥塞控制
- 验收：Chrome/Firefox/Safari 互通；弱网 20% 丢包；端到端 <500ms。

### Phase 4：SRT + WebTransport（2–3 个月）

- SRT：握手、ARQ、加密、丢包重传、caller/listener
- WebTransport：HTTP/3、QUIC、证书、双向流
- 验收：ffmpeg SRT 推拉；浏览器 WebTransport 播放；公网丢包测试。

### Phase 5：QUIC 集群（3 个月）

- Quinn 媒体 Relay、控制面 QUIC
- 种子发现、心跳、Healthy/Suspect/Offline
- 会话目录、发布/播放分配、重定向
- Origin 故障自动切换、Relay 重试、级联 Edge
- 验收：1 Origin + 3 Edge；Origin kill 后 5s 内切换；10k 并发；网络分区恢复。

### Phase 6：插件系统（2–3 个月）

- 稳定 Rust ABI / C ABI、动态库加载、版本校验
- 插件生命周期、配置热更新、权限/沙箱
- 内置协议插件化重构
- 验收：第三方插件不改主程序加载；故障隔离；ABI 兼容测试。

### Phase 7：无锁 RingBuffer + 性能（2 个月）

- SPMC ring、原子序号、ArcSwap 订阅者列表
- 对象池、零拷贝 Bytes、背压
- 基准：单流 10k 订阅、P99 延迟、内存/CPU
- 验收：与当前实现对比报告；无数据竞争（Loom/Miri）。

### Phase 8：商业化（持续）

- 版本/许可、SLA、支持流程、安全响应
- 部署（K8s/Helm）、多租户、计费
- 安全评估、渗透测试、合规
- 备灾/灰度/回滚
- 验收：商业合同、SLA、灾备演练、客户 PoC。

## 2. 优先级建议

先做 **Phase 1**，因为它直接产生可售卖价值（HLS/录制/Admin），且为后续协议提供统一媒体管线。WebRTC/GB28181 并行立项，但需要专人。

## 3. 团队配置

| 角色 | 人数 |
|---|---|
| 流媒体内核/协议 | 3–4 |
| WebRTC/SRT | 2 |
| 集群/基础设施 | 2 |
| 前端/Admin | 1–2 |
| QA/自动化/压测 | 1–2 |
| 安全/运维 | 1 |

## 4. 风险

- 协议互操作和边缘 case 远超文档；
- WebRTC/GB28181 需要真实设备与网络环境；
- QUIC 集群故障切换需要混沌测试；
- 商业替换涉及许可、专利、SLA、安全认证，不仅是代码；
- Monibuca v6 闭源，不能以复制源码方式追赶，只能清洁室实现。
