# m7s-rs 对标 Monibuca V6：差距清单与执行计划

> 对标目标：在协议、插件、数据面、集群、运维与商业化能力上，达到 Monibuca V6
> 的可替代水平（“完全一致”以可重复的兼容性验收为准，而不是仅看功能名称）。
>
> 参考：
> - [Monibuca V6 简介](https://monibuca.com/guides/introduction/)
> - [Monibuca V6 架构（26 插件 crate）](https://monibuca.com/architecture-overview.html)
> - [Monibuca 官网](https://monibuca.com/)
> - [v6 永久免费公告](https://mp.weixin.qq.com/s?__biz=MzAxNzg5MDIzMg==&mid=2247499481&idx=8&sn=cbedb4d9a49b517abb89561a88829561)

## 1. 结论摘要

当前 m7s-rs 已经是一个**可运行的协议原型/轻量服务器**：
RTMP、HTTP-FLV、HLS、RTSP、GB28181、WHEP/WHIP、SRT、WebTransport、
ONVIF、QUIC 集群、动态插件 ABI、许可证+席位均已打通，并有单元/集成测试。

但与 Monibuca V6 相比，仍缺少**四类能力**：

1. **架构底座**：不是 workspace + codec/sdk 分层，没有 lock-free RingBuffer、
   Arc 零拷贝帧、每流 Dispatcher、TaskSystem、RoomService。
2. **8 个媒体/设备/房间插件**：MP4、Snap、SEI、Mix、Transcode、Crypto、
   HomeKit、V4L2、ALSA、Live、Meeting 等尚未实现。
3. **运维与生态**：Admin 后台、Web SDK、gRPC、数据库、JWT/用户、SSE 监控、
   Report 三级上报、LogRotate、Crontab、Debug、Test 缺失。
4. **协议深度与集群**：HLS LL/fMP4/加密、RTSP Digest/VLC 稳定性、WebRTC TURN、
   GB28181 完整信令、SRT 完整 ARQ、集群心跳/分配/无缝切换仍不完整。

**当前不能宣称“与 V6 完全一致”**；本文给出到 V6 等价能力的阶段计划。

## 2. 当前已完成（有验证）

| 领域 | 已实现 | 验证 |
|---|---|---|
| RTMP | 推流/拉流、AMF0、chunk | FFmpeg 推拉 |
| HTTP-FLV | 长连接、GOP 缓存 | FFmpeg/VLC |
| HLS | MPEG-TS 分片、播放列表 | FFmpeg 拉流 |
| RTSP | TCP/UDP、RTP/RTCP、A/V | FFmpeg 验收 |
| GB28181 | SIP 注册/心跳、PS/RTP | 代码级/单元 |
| WebRTC | WHEP 单会话 A/V、WHIP H.264 | Chrome 147 帧 + 音频 |
| SRT | HSv5、MPEG-TS demux、AES | FFmpeg 推流 |
| WebTransport | QUIC/HTTP3、datagram TS | Edge 客户端 |
| ONVIF | Device/Media/Discovery、WS-Security、PTZ 响应 | curl/PowerShell |
| 集群 | HTTP+QUIC 中继、控制面、发现、重连 | 双节点/字节增长 |
| 插件 | 26 项注册表、启停、C ABI 动态加载 | `/api/plugins`、DLL |
| 商业化 | HMAC 许可证、强制、席位限制 | 401/RTMP 拒绝 |
| 数据面 | `Arc<FlvTag>`、broadcast、GOP 缓存 | 单元/集成 |

## 3. V6 的 26 个插件 crate 对照

V6 官方架构列出 26 个独立 crate：

```text
协议: RTMP RTSP FLV HLS WebRTC SRT WebTransport GB28181 ONVIF
媒体: MP4 Snap SEI Mix Transcode Crypto
工具: Debug LogRotate Report Crontab Test Cluster
设备: HomeKit V4L2 ALSA
房间: Live Meeting
```

| V6 插件 | m7s-rs 现状 | 差距 |
|---|---|---|
| RTMP | ✅ 基础 | 扩展时间戳、ack、aggregate、chunk 复用 |
| RTSP | ✅ 基础 | Digest、VLC 长稳、TCP 重连、RTP 周期 |
| FLV | ✅ | 基本对齐 |
| HLS | ✅ TS | LL-HLS、fMP4、加密、多码率 |
| WebRTC | ✅ WHEP/WHIP | TURN、Simulcast/SVC、ICE 重连 |
| SRT | ✅ | 完整 ARQ/丢包/带宽、调用方模式 |
| WebTransport | ✅ datagram | 可靠流、浏览器端、媒体映射标准 |
| GB28181 | ✅ 基础 | 完整 Catalog/Invite/PTZ/录像/回放 |
| ONVIF | ✅ 基础 | PTZ 状态、Media2、事件、真实设备 |
| **MP4** | ❌ | MP4 录制/点播/索引 |
| **Snap** | ❌ | 截图/缩略图 |
| **SEI** | ❌ | SEI 解析/注入 |
| **Mix** | ❌ | 多流混流/画布合图 |
| **Transcode** | ❌（仅 WHEP AAC→PCMU 外部 FFmpeg） | 转码插件、软/硬编、多档 |
| **Crypto** | ❌（仅 SRT 加密） | HLS AES、DRM、签名 URL |
| **Debug** | ❌ | pprof/堆/协程/抓包 |
| **LogRotate** | ❌ | 日志轮转/归档 |
| **Report** | ❌ | 三级上报/监控 |
| **Crontab** | ❌ | 定时任务 |
| **Test** | ❌ | 内置压测/自检 |
| **Cluster** | ✅ 基础 | 心跳、分配、无缝切换、三级 |
| **HomeKit** | ❌ | HomeKit 摄像头桥 |
| **V4L2** | ❌ | Linux 摄像头采集 |
| **ALSA** | ❌ | Linux 音频采集 |
| **Live** | ❌ | 房间、连麦、礼物、PK、机器人 |
| **Meeting** | ❌ | 会议房间/多人 |

当前 `/api/plugins` 的 26 项是**内部模块映射**，不是 V6 的 26 个独立 crate；
需要按上表重构命名与边界。

## 4. 架构与性能差距

| 维度 | Monibuca V6 | m7s-rs | 影响 |
|---|---|---|---|
| 代码组织 | workspace + codec/sdk/config + 26 crate | 单 crate | 插件边界不清、无法独立发布 |
| 帧模型 | AVFrame/Arc 零拷贝、Track | `Arc<FlvTag>` | 协议耦合 FLV，转码/混流困难 |
| 缓冲 | lock-free RingBuffer ~100ns | `RwLock<VecDeque>` + broadcast | 高并发锁竞争、延迟抖动 |
| 分发 | 每流 Dispatcher 单读多播 | broadcast channel | 无背压策略、无关键帧重发 |
| 动态插件 | IoC/EngineContext/SDK | C ABI 单插件 | 无法注册 HTTP/事件/配置 |
| 房间 | RoomService 内置 | 无 | 无法做直播/会议业务 |
| 任务 | TaskSystem 取消树 | tokio spawn | 生命周期/取消不统一 |
| 管理面 | Admin/Web SDK/gRPC/DB | 只读 HTTP | 不可运营 |
| 集群 | Origin-Edge/心跳/分配/切换 | QUIC 中继+基础发现 | 不能生产级容灾 |
| 性能 | 10K+ 流、100ns | 未统一基准 | 无法对标 |

## 5. 执行计划（P8–P14）

> 估算是 1–2 名熟悉 Rust/媒体的工程师、全职投入的工作量；
> 每个阶段以“可重复测试 + 可观测指标”收口，不以代码合并收口。

### P8 架构底座（2–3 周）

目标：把单 crate 重构成可承载 26 插件的底座。

- 建立 workspace：
  ```text
  crates/codec
  crates/sdk
  crates/config
  crates/engine
  plugins/<26>
  ```
- 定义 `AVFrame`、`Track`、`PublisherApi`、`SubscriberApi`、
  `StreamManagerApi`、`Demuxer/Muxer`；
- 实现 lock-free SPMC RingBuffer（单写多读、Arc 帧、~100ns 目标）；
- 每流 Dispatcher；
- TaskSystem（取消树、超时、优雅关闭）；
- 插件 trait + EngineContext（HTTP/Event/Config/Stream 注入）。

验收：

- 帧写入微基准 P50/P99；
- 1 万订阅者压测无锁竞争；
- 现有协议测试全部迁移通过。

### P9 媒体插件（3 周）

- MP4：录制、索引、点播、fMP4；
- Snap：JPEG/PNG 截图、缩略图；
- SEI：解析/注入/透传；
- Mix：多流画布合图；
- Transcode：FFmpeg 软编/硬编、多档、GOP 对齐；
- Crypto：HLS AES-128、签名 URL、密钥轮换。

验收：

- 录制文件 ffprobe 正确、可点播；
- 截图与源帧一致；
- 混流/转码输出可被 HLS/FLV 播放；
- 加密分片无法裸播、密钥鉴权通过。

### P10 设备与房间（3–4 周）

- V4L2/ALSA（Linux）采集；
- HomeKit 摄像头桥；
- Live 房间：房间生命周期、连麦、混流、礼物、PK、机器人、虚拟观众；
- Meeting 房间：多人会议、订阅关系、布局。

验收：

- 虚拟摄像头/声卡采集推流；
- 两房间连麦/混流可播放；
- 房间 API 与 Web SDK 联调。

### P11 运维与生态（3 周）

- Admin React 后台（登录、流管理、插件配置、日志、监控）；
- Web SDK（WHIP/WHEP/FLV/HLS 播放器）；
- gRPC（复用 AppFacade）；
- 数据库（SQLite/Postgres）+ 用户/JWT；
- SSE 监控、Report 三级上报、LogRotate、Crontab、Debug、Test；
- `/api/*` V5Compat 与 `/v6/api/*` 双表面。

验收：

- 官方风格 API 兼容测试；
- Admin 端到端操作；
- gRPC 与 HTTP 行为一致；
- 日志轮转/定时任务可观测。

### P12 协议深度（3 周）

- HLS LL-HLS/fMP4/加密/多码率；
- RTSP Digest、VLC 长稳、RTP 周期、TCP 重连；
- WebRTC TURN、Simulcast/SVC、ICE 重连；
- GB28181 完整 Catalog/Invite/PTZ/录像/回放；
- SRT 完整 ARQ/带宽/调用方；
- WebTransport 可靠流 + 浏览器端。

验收：

- VLC/Chrome/FFmpeg 兼容矩阵；
- 丢包/弱网/重连测试；
- 真实 ONVIF/GB28181 设备（或模拟器）互通。

### P13 集群与容灾（2 周）

- 心跳、节点注册、会话同步、分配算法；
- Edge 无缝切换（播放器不中断）、时间轴连续；
- 多机房/多副本；
- 集群级监控与告警。

验收：

- 杀 Origin、播放器持续播放；
- 扩容/缩容不中断；
- 脑裂/分区测试。

### P14 性能与等价验收（2 周）

- 统一压测基准：1K/5K/10K 流、帧延迟 P50/P99、CPU/RSS/带宽；
- 与官方 V6 同机同场景对比；
- 26 插件逐项验收表；
- API/协议兼容性回归；
- 文档、部署、升级、回滚。

验收：

- 10K+ 并发流；
- 帧写入 ~100ns 量级（同机对比）；
- 26/26 插件通过；
- 关键 API 与 V6 行为一致（以官方二进制为 oracle）。

## 6. “完全一致”的验收定义

由于 V6 闭源，无法保证二进制/逐行一致。可执行的“完全一致”定义为：

1. **协议互通**：RTMP/RTSP/FLV/HLS/WebRTC/SRT/GB28181/WebTransport/ONVIF
   与官方 V6、FFmpeg、VLC、Chrome、真实设备互通；
2. **插件对等**：V6 的 26 个 crate 名称/职责逐项有实现与验收；
3. **API 兼容**：`/api/*` 与 `/v6/api/*` 的关键路径、字段、错误码通过
   录制回放对比；
4. **性能达标**：同机 10K 流、帧延迟、CPU/RSS 在官方口径同一量级；
5. **运维可用**：Admin、Web SDK、gRPC、DB、监控、日志、定时任务可演示；
6. **容灾**：Origin 故障时播放器无感切换。

## 7. 优先级建议

若目标是“尽快可替代”：

1. P8 数据面（决定上限）；
2. P9 MP4/Snap/Transcode（商业场景刚需）；
3. P11 Admin/Web SDK（可交付）；
4. P12 RTSP/HLS/WebRTC 深度（兼容性）；
5. P10 房间/设备（按行业场景）；
6. P13 集群容灾；
7. P14 性能对标。

## 8. 当前不应宣称的能力

- “26 插件与 V6 完全一致”（当前是内部模块映射）；
- “lock-free/零拷贝/100ns”（当前 RwLock+broadcast）；
- “10K+ 并发流”（无统一基准）；
- “生产级 QUIC 集群容灾”（仅基础重连）；
- “Admin/Web SDK/gRPC/DB/房间”（未实现）。
