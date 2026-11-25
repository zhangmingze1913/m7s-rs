# Monibuca V5 → V6 演进对比与本项目定位

参考：

- [Monibuca v6 简介](https://monibuca.com/guides/introduction/)
- [Monibuca v6 运行时架构](https://monibuca.com/concepts/architecture/)
- [Monibuca v6 编译期架构（26 插件 crate）](https://monibuca.com/architecture-overview.html)
- [Monibuca v5 源码（Gitee）](https://gitee.com/m7s/monibuca)

## 1. V5 → V6 的核心变化

| 维度 | V5（Go） | V6（Rust） |
|---|---|---|
| 语言/内存 | Go GC、手动内存管理局部 | Rust，编译期内存安全 |
| 代码组织 | `pkg/` + `plugin/` Go package | Workspace：codec、sdk、config-framework、26 个插件 crate |
| 数据面 | Publisher/Subscriber、流管理 | RingBuffer、ArcSwap、Bytes 零拷贝、Dispatcher 单读广播 |
| 协议 | RTMP/RTSP/FLV/HLS/WebRTC/SRT/GB28181 等 | 同上 + WebTransport，插件化 |
| 集群 | 级联/房间等 | Origin-Edge、QUIC Relay、心跳三态、分配/重定向 |
| 插件 | Go plugin/编译期 | 26 个独立 crate，动态加载/SDK |
| 运维 | Admin/监控 | Admin、SSE 监控、三级上报、Web SDK |
| 性能目标 | 高吞吐 | ~100ns 帧写入、10K+ 流、低延迟 |
| 许可 | AGPL | 闭源商业（预编译） |

## 2. V6 架构要点

```text
外部客户端（Web SDK/App/FFmpeg/OBS/GB28181/Admin）
        ↓
基础设施层：HTTP / gRPC / DB / Config / Event / FFmpeg / QUIC / Auth
        ↓
引擎内置：StreamManager（Publisher/Subscriber/Track/RingBuffer）
          PluginManager（IoC/EngineContext/动态加载）
          RoomService、TaskSystem、CoreAdapters
        ↓
26 个功能插件 crate（协议/媒体/工具/设备/房间）
        ↓
monibuca-sdk + codec + config-framework（基础 crate）
```

HTTP 面：

- `/api/*`：V5 兼容；
- `/v6/api/*`：V6 原生 REST；
- `/zego/api/*`：ZEGO 兼容（feature-gated）；
- gRPC：共用 AppFacade。

## 3. m7s-rs 当前映射

| V6 能力 | m7s-rs | 差距 |
|---|---|---|
| Rust 引擎 | ✅ 单 crate | 未拆 workspace/codec/sdk |
| RTMP | ✅ 基础 | 扩展时间戳/ack/aggregate 不全 |
| HTTP-FLV | ✅ | — |
| HLS | ✅ H.264/AAC | LL-HLS/fMP4/加密未做 |
| TLS | ✅ HTTPS（rustls） | RTMPS/mTLS 未做 |
| 录制 | ✅ FLV | MP4/配额/回放索引未做 |
| Admin | ✅ 只读 | 写操作/鉴权/监控未做 |
| 集群 | ✅ HTTP-FLV Relay | 非 QUIC，无心跳/故障切换 |
| RingBuffer | ❌ RwLock + GOP | 无锁/零拷贝未做 |
| 插件 | ❌ | 无 ABI/动态加载 |
| RTSP/WebRTC/SRT/GB28181/WebTransport | ❌ | 未实现 |
| 26 插件 crate | ❌ | 仅 7 个媒体模块 |
| 10K 流 | ❌ | 仅 1000 HTTP-FLV 实测 |

## 4. 结论

m7s-rs 是 **V6 架构思想的轻量参考实现**，不是 V6 的等价替代：

- 已完成“可运行媒体链路 + HLS + TLS + 录制 + 集群 Relay + 1k 压测”；
- 未完成“26 协议插件、无锁 RingBuffer、QUIC 集群、WebRTC/GB28181/SRT、商业化”。

V5→V6 的启示不是“换语言”，而是：**codec/SDK 分层、插件 crate 化、无锁数据面、QUIC 集群、Admin/监控一体化**。这些是后续阶段的架构目标。
