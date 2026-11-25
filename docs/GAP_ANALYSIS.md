# m7s-rs vs Monibuca V6：功能与性能差距

## 1. 当前基线

| 项 | 当前 |
|---|---|
| 代码 | ~4,741 Rust LOC，单 crate |
| 测试 | 16 单元 + 7 集成 |
| Release | ~4.76 MB |
| 协议 | RTMP 基础、HTTP-FLV、HLS（部分）、RTSP（TCP/UDP 基础）、GB28181 SIP 注册 |
| 集群 | HTTP-FLV Relay，非 QUIC |
| TLS | HTTPS/RTMPS/mTLS |
| 压测 | 1000/5000 HTTP-FLV 通过；10000 约 5701/10000 |
| 延迟/CPU/P99 | 未采集 |
| 零拷贝/无锁 | 未实现 |

## 2. 功能差距

| 能力 | Monibuca V6 | m7s-rs | 差距 |
|---|---|---|---|
| RTMP | 完整 | 基础推/拉 | 扩展时间戳、ack、aggregate、chunk 复用 |
| HTTP-FLV | 完整 | ✅ | 基本对齐 |
| HLS | 完整/LL-HLS | 部分 | 告警、LL-HLS、fMP4、加密、多码率 |
| RTSP | 完整 | 基础 | VLC 稳定播放、Digest、UDP 完整、RTCP 周期、TCP 重连 |
| WebRTC | WHIP/WHEP | ❌ | ICE/DTLS/SRTP/拥塞控制 |
| SRT | ✅ | ❌ | 握手/ARQ/加密 |
| GB28181 | ✅ | SIP 注册/心跳 | Digest、Catalog、INVITE、RTP/PS |
| WebTransport | ✅ | ❌ | HTTP/3/QUIC |
| ONVIF | ✅ | ❌ | 发现/能力/Profile |
| MP4/Snap/SEI/Transcode | ✅ | ❌ | 媒体处理插件 |
| 插件系统 | 26 crate + 动态加载 | ❌ | ABI/SDK/沙箱 |
| 集群 | QUIC Origin-Edge、心跳、故障切换 | HTTP Relay | QUIC、心跳、自动切换 |
| Admin/Web SDK | 完整 | 只读 Admin | 写操作、监控、播放器 |
| 监控 | SSE/三级上报 | 基础 metrics | 指标/追踪/告警 |
| 数据面 | RingBuffer/ArcSwap/Bytes 零拷贝 | RwLock + broadcast + VecDeque | 无锁、零拷贝、背压 |
| 压测 | 官方称 10K+ | 5K 通过，10K 未通过 | 容量与调优 |

## 3. 性能差距

官方 V6 公开口径：

- ~100ns 帧写入延迟；
- 10K+ 并发流；
- 25+ 插件、8+ 协议；
- lock-free RingBuffer、零拷贝转发。

这些是厂商口径，当前无法独立复现。当前项目实测：

| 指标 | 当前 | 说明 |
|---|---|---|
| 1000 HTTP-FLV | 1000/1000 | 2.6s，7.3MB RSS，CPU 44.5% |
| 5000 HTTP-FLV | 5000/5000 | 6.23s，CPU 57.7%，P99 12.3ms |
| 10000 HTTP-FLV | **10000/10000** | 9.27s，CPU 59.5%，P99 14.4ms |
| 帧转发延迟 | 未测 | 无 P50/P99 |
| CPU/内存/带宽 | 未测 | 无压测报告 |
| 零拷贝 | 否 | `Arc<FlvTag>` + broadcast，仍有队列/RwLock |
| 无锁 | 否 | `RwLock<VecDeque>` |
| 背压 | broadcast lag 丢弃 | 无关键帧请求/重发 |

结论：当前不是“接近 V6 性能”，而是**功能原型 + 有限压测**。没有统一基准，不能把 5K 与官方 10K 直接比较。

## 4. 架构差距

V6：

```text
codec + sdk + config-framework
        ↓
26 插件 crate（协议/媒体/工具/设备/房间）
        ↓
StreamManager / RingBuffer / ArcSwap / QUIC
        + Admin / Web SDK / 监控
```

当前：

```text
单 crate
  ├─ rtmp / rtsp / hls / gb28181
  ├─ stream: RwLock + broadcast
  ├─ cluster: HTTP-FLV relay
  └─ http/admin
```

关键差距：

1. 没有 `MediaFrame/Track` 统一模型，协议各自处理 FLV tag；
2. 没有 lock-free RingBuffer/对象池/零拷贝；
3. 没有插件 ABI、动态加载、SDK；
4. 集群不是 QUIC，没有心跳三态/故障切换；
5. Admin/监控/Web SDK 不完整；
6. 没有统一基准与 SLO。

## 5. 结论

- 当前适合做 **协议与架构验证原型**；
- 不适合宣称“对标 V6”或“商业替换”；
- 下一步必须先补**数据面与基准**，再堆协议数量；否则 26 个插件会建立在错误内核上。
