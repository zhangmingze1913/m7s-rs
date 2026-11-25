# Monibuca V6 对标分析与规划

Date: 2026-11-16

## 1. 性能实测（m7s-rs release）

| 指标 | 结果 | 条件 |
|---|---|---|
| HTTP-FLV 10K viewers | 10000/10000, 16.98s | 单流 `live/test`，release |
| 进程 baseline RSS | 6.7 MB | 空载 |
| 10K viewers 峰值 RSS | 515.6 MB | 约 51 KB/viewer |
| 10K 结束后 RSS | 10.9 MB | 连接释放 |
| 累计 CPU | 6.2s（约 12s 窗口） | 单机 |
| RingBuffer 写入 | 117.2 ns/write | 池化 Arc，1M 次 |

### 与 V6 公开口径对比（谨慎）

- V6 宣称 ~100ns/帧写入：我们 117.2ns，同一量级。
- V6 宣称 10K 并发：我们验证的是 **10K viewers on 1 stream**；
  **10K 独立 stream** 尚未验证，不能等同。
- 内存/CPU 数据是单机 Windows 实测，不是 V6 官方基准环境。

## 2. 功能对齐矩阵

| 域 | m7s-rs | V6 目标 | 差距 |
|---|---|---|---|
| FLV/RTMP/HTTP-FLV | ✅ | ✅ | 基本对齐 |
| HLS TS | ✅ | ✅ | 对齐 |
| HLS fMP4 A/V | ✅ 干净解码 | ✅ | 对齐基础播放 |
| LL-HLS | PART/PRELOAD/阻塞 | ✅ | 缺 PART 预加载推送/完整序列 |
| RTSP | ✅ TCP/UDP A/V | ✅ | 对齐核心 |
| SRT | ✅ | ✅ | 对齐核心 |
| WebTransport | ✅ datagram/stream | ✅ | 缺浏览器可靠流 |
| WHEP/WHIP | ✅ H264/PCMU | ✅ | 对齐核心 |
| Simulcast RID | ✅ 手动/自动 | ✅ | 缺精确带宽 |
| REMB | ✅ 接线到 RID | ✅ | 已接线 |
| TWCC | ⚠️ 丢包解码 | ✅ 带宽估计 | 缺完整估计 |
| SVC | ❌ | ✅ | 需编码器层 |
| GB28181 | ✅ SIP/PS/RTP/回放/PTZ 信令 | ✅ | 缺 MP4 录像源/完整回放控制 |
| ONVIF | ✅ Device/Media/Media2/Events | ✅ | 缺 Media2 全量/事件推送 |
| Cluster | ✅ HTTP/QUIC | ✅ | 缺故障切换压测 |
| Admin/Web SDK | ✅ 基础 | ✅ | 缺业务配置/多租户 |
| gRPC | ✅ 基础 | ✅ | 方法少 |
| DB | ✅ 文件/MySQL 用户 | ✅ | 缺流/房间元数据 |
| Live/Meeting | ✅ 房间/礼物/PK/发布 | ✅ | 缺业务完整度 |

## 3. 规划

### P0（性能与阻断项）

1. **10K 独立流**压测（不是 viewers），记录 CPU/RSS/失败率；
2. TWCC 从丢包启发式升级为发送历史 + 到达时间带宽估计；
3. SVC 方案落地：外部编码器或明确不支持声明；
4. Cluster 故障切换压测。

### P1（协议深度）

1. LL-HLS 完整 PART 序列/预加载推送；
2. GB28181 MP4 录像源、暂停/恢复/倍速完整；
3. ONVIF 事件订阅；
4. HLS AES-128。

### P2（业务/控制面）

1. Admin 配置写操作、插件参数；
2. gRPC 扩展 Stream/Plugin/Cluster；
3. 房间业务持久化；
4. 多租户/配额。

## 4. 验收门禁

- 每个 P0 项必须有 release 实测数据；
- 功能项必须有自动化测试 + 真实协议验证；
- 不把不可实现项标为完成；
- 每轮变更执行 clippy/test/release。

## 5. 结论

性能已达 V6 同量级（10K viewers、117ns ring），但“完整对标”仍未达成；
SVC、TWCC 完整带宽估计、10K 独立流是当前最大缺口。

## 10K independent streams (debug harness)

`scripts/load_streams.cjs` + `M7S_DEBUG_STREAMS=1`:

```text
streams=10000 created_ms=9180 viewers_ok=10000 failed=0 elapsed_s=13.53
process RSS=1204.4 MB, CPU=18.7 s
```

Caveat: streams carry no media (debug create endpoint); this measures session
/table scaling and one HTTP-FLV connection per stream, not 10K encoders.