# Monibuca V6 对标复核（2026-11-17）

官方资料：

- [简介](http://www.monibuca.com/guides/introduction/)
- [Dispatcher 架构](https://www.monibuca.com/en/concepts/dispatcher/)
- [系统架构](https://monibuca.com/concepts/architecture/)
- [版本/下载](https://monibuca.com/guides/download/)
- [官网性能声明](http://monibuca.com/en/)

## 1. 官方性能声明

| 指标 | V6 官方 |
|---|---|
| 单帧写入 | ~100 ns |
| 订阅者读取 | ~50 ns |
| 单节点并发流 | 10,000+ |
| 单流并发订阅者 | 10,000+ |
| 分发 | Dispatcher 单读广播 O(1)+O(N) |
| 慢订阅者 | bounded channel（默认 150），满则丢帧 |
| 大规模流 | >100 流可启用 DispatcherPool |

## 2. m7s-rs 实测（最新）

| 指标 | 实测 | 条件 |
|---|---|---|
| Ring 写入 | 117.2 ns | 池化 Arc |
| 单流 10K viewers | 10000/10000, **15.54s** | Dispatcher |
| 10K 真实 RTMP 流 | **10000 条，s0 bytes 持续增长** | 40 FFmpeg tee |
| 10K 真实流 RSS | **≈2131.8 MB** | 单机 |
| 10K 真实流 CPU | ≈65.9 s 累计 | 单机 |
| 100 真实流 | 100/100 | 单机 |
| 100K/1M 连接 | 未验证 | 单机端口约 16–17K |

## 3. 功能差异（更新）

| 能力 | V6 | m7s-rs | 状态 |
|---|---|---|---|
| Lock-free Ring + Arc | ✅ | ✅ 117ns | 同量级 |
| Dispatcher 单读广播 | ✅ | ✅ per-stream Dispatcher | 已对齐核心 |
| bounded channel 150 + drop | ✅ | ✅ | 已对齐 |
| DispatcherPool | ✅ | ✅ `M7S_DISPATCHER_WORKERS` | 已对齐 |
| 10K+ streams | ✅ | ✅ 10K 真实流 | 已对齐（同内容） |
| 10K+ subscribers | ✅ | ✅ 10K viewers | 已对齐 |
| 26 plugin crates | ✅ | ✅ 26 | 数量对齐 |
| 5 foundation crates | ✅ | ❌ 3 个 | 中 |
| WASM 沙箱插件 | ✅ | ✅ wasmi + 文件/启用/链 | 基本对齐 |
| WASM 帧 ABI | ✅ | ✅ `transform_av` | 基本对齐 |
| SVC | ✅（官方） | ❌ | 大 |
| TWCC 带宽估计 | ✅ | ⚠️ 粗粒度 | 中 |
| HLS TS/fMP4/LL | ✅ | ✅ 基础，LL 不完整 | 中 |
| GB28181 | ✅ | ✅ 信令/PS/RTP/回放，缺 MP4 源 | 中 |
| RTSP/SRT/WebTransport | ✅ | ✅ 核心 | 对齐 |
| WHEP/WHIP | ✅ | ✅ H264/PCMU | 核心对齐 |
| Cluster | 企业版 | ✅ HTTP/QUIC 基础，缺故障切换 | 中 |
| Room/Live 业务 | ✅ 完整 | ⚠️ 基础房间/礼物/PK | 大 |
| React Admin | ✅ | ⚠️ 简易 HTML | 大 |
| 版本/授权 | 50/1000/不限 | ⚠️ 自有 license | 中 |

## 4. 仍存在的关键差异

### 4.1 SVC（最大功能缺口）

V6 官方声明 SVC；我们只有 H.264 单层：

- 无 SVC 编码器；
- `webrtc 0.11` 无空间层提取；
- `/api/webrtc/capabilities` 仍 `svc:false`。

### 4.2 TWCC

已有“包数×1200B/时间差”的粗估并驱动 RID，但没有发送历史/到达时间
精确估计，弱于 V6 的完整 TWCC。

### 4.3 规模验证

- 10K 真实流已通过（同内容、单机 tee）；
- 100K/1M 未验证：单机客户端端口约 16–17K；
- 需要多台压测机/多源 IP。

### 4.4 业务层

V6：内置 Room 服务、Live 连麦/礼物/PK/机器人、React Admin；
我们：基础房间、礼物、PK、简易 Admin。

### 4.5 工程结构

V6 有 5 个 foundation crate；我们是 3 个（sdk/codec/config）。
WASM 已用 wasmi，但还不是 V6 级别的插件市场/热加载体系。

### 4.6 协议深度

- HLS LL 的 PART 推送/预加载不完整；
- GB28181 缺 MP4 录像源与完整暂停/倍速；
- Cluster 缺 origin 故障自动切换。

## 5. 修复规划

### P0（对齐核心能力）

1. **SVC**：接入外部 SVC 编码器（如 OpenH264 SVC/硬件）或明确不支持；
2. **TWCC 完整估计**：发送历史 + 到达时间 + 带宽趋势；
3. **100K 分布式压测**：多压测机，每台 ≤10K 连接；
4. **Cluster 故障切换**：origin 宕机后 edge 切换。

### P1（协议/工程）

1. HLS LL PART 推送与预加载；
2. GB28181 MP4 录像源、暂停/恢复/倍速；
3. 5 个 foundation crates 拆分；
4. WASM 插件热加载/市场 ABI。

### P2（业务/授权）

1. Room 内置服务与 Live 完整业务；
2. React Admin / Web SDK；
3. 官方 edition（50/1000/不限）与集群授权；
4. gRPC/DB 扩展。

## 6. 结论

已对齐：Ring 同量级、Dispatcher/Pool、10K 真实流、10K viewers、WASM 基础。
最大未对齐：**SVC、TWCC 精度、100K/1M 规模、Room/Admin 业务**。
未实现项不标为完成。
