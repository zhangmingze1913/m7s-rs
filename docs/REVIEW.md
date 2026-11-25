# 代码、架构与集群复盘

## 1. 已完成

- RTMP 握手、chunk 编解码、AMF0 命令、publish/play。
- HTTP-FLV（GOP 缓存 + 直播广播）。
- Origin/Edge：会话目录、Edge 回源、HTTP-FLV Relay、Relay 列表。
- HTTP API、配置、鉴权 key。
- 6 个单元测试 + 2 个真实 TCP 集成测试。
- Clippy 无警告，release 构建通过。

## 2. 已发现问题与修复

| 问题 | 影响 | 修复 |
|---|---|---|
| Edge Relay 首次回源把 FLV 头/tag 留在预读缓冲区未消费 | 首次播放一直无画面 | `pull_flv` 先消费 residual 再进入读循环 |
| Relay 元数据未登记 | `/relay/sessions` 为空 | 回源任务启动时写入 RelayInfo |
| RTMP chunk 的 2019 版本判断用错字节 | 握手后解析错误 | 使用 body property bit14 判断 |
| AMF0 object end 未消费 0x09 | 后续命令错位 | 读取结束 marker |
| FLV 时间戳高字节解析 | 长直播时间戳错误 | 按 FLV 规范拼接 |
| 集群用 unsafe 指针共享 Cluster | 内存安全风险 | 改为 `Arc<Mutex>` 克隆 |

## 3. 架构复盘

### 优点

- 媒体链路短：RTMP → FlvTag → Stream → HTTP-FLV，无转码/拷贝放大。
- GOP 缓存使新观众快速出画；broadcast 解耦发布/订阅。
- 集群控制面与媒体面分离，Edge 回源可独立测试。
- 真实 TCP 测试覆盖握手/推流/回源，不只是函数级 mock。

### 风险

1. **RTMP 不完整**：扩展时间戳写、aggregate、ack、chunk stream 复用、AMF3 未覆盖。
2. **集群无 QUIC**：HTTP-FLV 回源延迟高于 QUIC，且无多路复用。
3. **无心跳/故障切换**：种子查询失败即跳过；Origin 下线后 Edge 不自动切源。
4. **无锁性不足**：Stream 使用 `RwLock<VecDeque>`，不是 Monibuca 的 lock-free RingBuffer。
5. **安全**：无 TLS/mTLS、API 鉴权、推流签名过期、限流。
6. **协议覆盖**：HLS/RTSP/WebRTC/SRT/GB28181 未实现。
7. **可观测性**：无 Prometheus/日志聚合/追踪。

## 4. 集群复测

已执行：

```text
Origin 127.0.0.1:51355（有 live/test）
Edge   127.0.0.1:51356
Edge 收到 /flv/live/test.flv
→ GET origin /cluster/api/sessions
→ GET origin /flv/live/test.flv
→ 解析 FLV header + tag
→ 写入 Edge Stream
→ 返回 Edge 客户端
```

断言：Edge 输出包含 `FLV` header 和 video tag；`/cluster/api/relay/sessions` 有记录（运行期）。

结论（当时）：**单 Origin + 单 Edge 的回源链路可用**；多 Edge、Origin 故障切换、QUIC Relay 尚未验证。
后续更新：QUIC relay 与 origin failover 已由 `tests/quic_failover.rs` 覆盖（2026-11-21）。

## 5. 下一阶段优先级

P0：

- RTMP 扩展时间戳/ack/aggregate；OBS/FFmpeg 长稳测试。
- 发布者断线重连与流替换。
- 集群心跳、Origin 故障切换、Relay 重试。
- TLS + 推流鉴权。

P1：

- HLS（TS/fMP4）。
- 无锁 RingBuffer/对象池。
- 指标与 Admin API。

P2：

- RTSP/GB28181、WebRTC、SRT、WebTransport。
- 插件化与动态加载。
