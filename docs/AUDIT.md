# 代码与架构审计（Phase 0）

## 1. 现状

| 项 | 值 |
|---|---|
| 代码 | ~3.5k Rust LOC |
| 单元测试 | 10 |
| 集成测试 | 2（RTMP→FLV、Origin→Edge） |
| 静态检查 | clippy `-D warnings` 通过 |
| Release | ~3 MB |
| 协议 | RTMP 基础、HTTP-FLV、HLS、FLV 录制、HTTP 集群 Relay |

## 2. 必须修复的生产问题

| 级别 | 问题 | 影响 | 建议 |
|---|---|---|---|
| P0 | 无 TLS/mTLS | 推流/API 明文 | rustls + 证书管理 |
| P0 | API 无鉴权 | 任意查询/控制 | Token/mTLS/ACL |
| P0 | RTMP 扩展时间戳写缺失 | 长直播时间戳回绕 | 按规范写 extended timestamp |
| P0 | 集群无心跳/故障切换 | Origin 下线即中断 | 心跳三态 + Relay 重试 |
| P1 | broadcast lag 直接丢帧 | 慢客户端花屏 | GOP 重发/关键帧请求 |
| P1 | 无背压/限流 | 大并发 OOM | bounded cache/连接数限制 |
| P1 | 无 RTMP ack/aggregate | OBS 长推流不稳定 | 补齐协议 |
| P1 | HLS 未实现 | 浏览器/移动端不可用 | Phase 1 |
| P2 | 无持久化/录制 | 无法回放 | Phase 1 |
| P2 | 无插件 ABI | 无法扩展 | Phase 6 |
| P2 | Stream 用 RwLock | 非无锁核心 | Phase 7 |

## 3. 架构问题

- `Stream` 的 GOP 缓存和 broadcast 是两套状态，极端并发下新订阅者可能拿到不完整 GOP；
- `Cluster::ensure_relay` 只保证“第一次回源”，没有多 Edge 协同和源选择；
- `http` 的 FLV 播放没有超时/断线重连语义；
- `RTMP server` 每个连接一个任务，缺少连接数上限和任务监管；
- 没有统一的媒体帧模型（目前直接 FLV tag），跨协议转封装会重复解析。

建议下一阶段引入：

```text
Protocol Demuxer -> MediaFrame(Audio/Video/Metadata) -> Stream/RingBuffer
                                     -> Protocol Muxer (FLV/TS/MP4/RTP/WebRTC)
```

## 4. 集群复测结论

已通过单 Origin + 单 Edge 回源测试：

- Edge 查询 sessions；
- 拉取 Origin FLV；
- 解析 header + tag；
- 写入本地 Stream；
- 返回客户端。

未通过/未验证：

- Origin 宕机后的自动切换；
- 多 Edge 并发回源去重；
- 网络分区；
- 10k 并发；
- QUIC Relay。

## 5. 结论

Phase 0 可作为技术验证原型，不可直接商业替换 Monibuca。商业化前必须完成 ROADMAP 的 Phase 1–8，并以压测、混沌测试、安全评估和客户 PoC 作为验收。
