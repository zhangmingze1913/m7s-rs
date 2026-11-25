# Monibuca V6 官方能力 vs m7s-rs 差异分析

Date: 2026-11-17

参考官方资料：

- [Monibuca 简介](http://www.monibuca.com/guides/introduction/)
- [Dispatcher 架构](https://www.monibuca.com/en/concepts/dispatcher/)
- [系统架构](https://monibuca.com/concepts/architecture/)
- [下载/版本限制](https://monibuca.com/guides/download/)
- [官网性能声明](http://monibuca.com/en/)

## 1. 官方性能声明

| 指标 | V6 官方 |
|---|---|
| 单帧写入 | ~100ns |
| 订阅者读取 | ~50ns |
| 单节点并发流 | 10,000+ |
| 单流并发订阅者 | 10,000+ |
| 分发 | Dispatcher 单次读取广播，O(1) read + O(N) broadcast |
| 慢订阅者 | bounded channel（默认 150），满则丢帧不阻塞 |
| 大规模流 | >100 流可启用 DispatcherPool（一致性哈希） |

## 2. m7s-rs 实测

| 指标 | 实测 | 条件 |
|---|---|---|
| Ring 写入 | **117.2 ns** | 池化 Arc，1M 次 |
| 单流 10K viewers | **10000/10000, 16.98s** | 单节点 |
| 10K 独立空流 + 1 viewer/流 | **10000/10000, 13.53s, RSS 1204MB** | 单节点 |
| 真实 RTMP 流 | 100/100（tee 推流） | 单节点 |
| 单机连接上限 | 约 16–17K | 客户端端口/accept 限制 |
| 100K/1M | 未验证 | 需分布式压测机 |

## 3. 架构差异（关键）

### 3.1 分发模型

- **V6**：每流一个 Dispatcher；单次 RingBuffer 读取，向所有订阅者的
  bounded channel 广播；慢订阅者丢帧；`ArcSwap<Vec<Subscriber>>` 管理列表。
- **m7s-rs**：每订阅者一个 `TagReader` 独立读 RingBuffer，N 个订阅者
  = N 次读；没有每订阅者 channel/丢帧计数。

影响：订阅者数很大时，我们读路径 O(N)，V6 为 O(1) 读取 + O(N) channel
发送；我们的 10K viewers 已通过，但 CPU/缓存行为与 V6 不同。

### 3.2 DispatcherPool

V6 支持 >100 流时按一致性哈希分 Worker；我们所有流共享 Tokio 调度，
未实现分片池。

### 3.3 插件沙箱

V6 支持 WASM 沙箱插件；我们是 `libloading` C ABI 动态库，无沙箱隔离。

### 3.4 房间/后台

V6 内置 Room 服务、Live 插件（连麦、礼物、PK、机器人、虚拟观众）、
React Admin；我们只有基础房间状态/礼物/PK 与简易 Admin。

### 3.5 版本与授权

V6 免费版 50 连接、专业版 1000、企业版不限且集群仅企业版；
我们的 license/seat 逻辑是自定义，未按官方 edition 对齐。

## 4. 功能差异矩阵

| 能力 | V6 | m7s-rs | 差距 |
|---|---|---|---|
| Lock-free ring + Arc | ✅ | ✅ 117ns | 同量级 |
| Dispatcher 单读广播 | ✅ | ❌ N-reader | 大 |
| bounded channel + 丢帧计数 | ✅ | ❌ | 大 |
| DispatcherPool | ✅ | ❌ | 大 |
| 26 plugin crates | ✅ | ✅ | 数量对齐 |
| 5 foundation crates | ✅ | ❌（3 个） | 中 |
| WASM 沙箱插件 | ✅ | ❌（C ABI） | 大 |
| 10K+ streams | ✅ | 10K 空流；100 真实流 | 中 |
| 10K+ subscribers | ✅ | ✅ 10K viewers | 对齐 |
| HLS fMP4/LL | ✅ | 基础 | 中 |
| WHEP/WHIP | ✅ | ✅ | 对齐核心 |
| SVC | ✅（官方声称） | ❌ | 大 |
| Cluster | 企业版 | ✅ 基础 | 中 |
| Room/Live 业务 | ✅ 完整 | 基础 | 大 |
| React Admin | ✅ | 简易 HTML | 大 |
| WASM 插件 | ✅ | ❌ | 大 |

## 5. 修复规划

### P0（架构与性能）

1. **实现 Dispatcher**：
   - 每流单 reader；
   - `ArcSwap<Vec<Subscriber>>`；
   - 每订阅者 bounded channel（150）；
   - 满则丢帧 + `dropped` 计数；
   - HTTP-FLV/RTSP/WHEP 等改从 channel 消费。
2. **DispatcherPool**：>100 流一致性哈希到 worker。
3. 10K 真实流分布式压测（多压测机）。

验收：单流 10K subscriber 的读次数从 N 降为 1；慢订阅者不阻塞；
丢帧计数可观测；10K viewers 性能不低于当前。

### P1（隔离与协议深度）

1. WASM 沙箱插件运行时；
2. foundation crates 拆分（sdk-macros/config-framework/config-macros）；
3. HLS fMP4/LL 完整、SVC 外部编码器方案；
4. 真实 10K 流压测。

### P2（业务/控制面）

1. Room 内置服务与 Live 完整业务；
2. React Admin / Web SDK 完整；
3. 官方 edition/license 对齐；
4. gRPC/DB 扩展。

## 6. 结论

性能写入延迟已对齐 V6 同量级（117ns vs ~100ns），10K viewers 已通过；
最大架构差异是 **Dispatcher 单读广播 + bounded channel**，其次是
DispatcherPool、WASM 沙箱、Room/Admin 业务。

未实现项不标为完成；优先级按 P0→P1→P2。

## Update 2026-11-17: Dispatcher implemented

- Dispatcher + bounded channel + dropped counter: ✅
- 10K HTTP-FLV: **10000/10000, 15.54s** (was 16.98s)
- DispatcherPool: ❌ still pending
## Update: DispatcherPool implemented

- DispatcherPool + consistent hashing: ✅ (`M7S_DISPATCHER_WORKERS`)
- 10K HTTP-FLV with 4 workers: 10000/10000, 15.54s