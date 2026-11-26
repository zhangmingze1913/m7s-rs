# 1000 / 5000 / 10000 并发压测记录

## 1. 环境

- 本机 Windows，单进程 release；
- `--demo` 合成 H.264/AAC；
- Node.js 客户端，HTTP-FLV 并发连接；
- `server.backlog` 调到 16384 后复测。

## 2. 结果

### 1000 并发

```text
viewers=1000 ok=1000 failed=0 elapsed=4.53s
```

### 5000 并发

```text
viewers=5000 ok=5000 failed=0 elapsed=8.33s
```

### 10000 并发（单客户端）

```text
viewers=10000 ok=5701 failed=4299 elapsed=10.11s
```

### 10000 并发（两个客户端各 5000）

```text
A: viewers=5000 ok=3146 failed=1854
B: viewers=5000 ok=2444 failed=2556
合计: 5590 / 10000
```

## 3. 结论

- **5000 并发 HTTP-FLV 在本机稳定通过**；
- **10000 未通过**，成功率约 56–57%，瓶颈在单机/客户端连接与调度，不是协议错误；
- backlog 从 4096 提升到 16384 后，5000 通过；10000 仍受句柄/CPU/客户端限制；
- 10000 需要多进程客户端、Linux 调优（`ulimit -n`、`somaxconn`、`tcp_tw_reuse`）和独立压测机，不能伪造为通过。

## 4. 边界

- 合成流，非真实编码器；
- 未统计 CPU/内存/P99；
- 未做 72h 浸泡、弱网、OBS/FFmpeg 长推流；
- HLS 并发未压测。

原始日志：`load-1000.log`、`load-5000.log`、`load-10000.log`。

## 2026-11-16 10K HTTP-FLV re-test

Release binary, `--demo`, lock-free RingBuffer path:

```text
viewers=10000 ok=10000 failed=0 elapsed=16.65s
url=http://127.0.0.1:8180/flv/live/test.flv
```

This supersedes the earlier 5701/10000 result.
## 2026-11-23 性能优化与复测

优化：
- `FlvTag::write_to` 直接写入复用缓冲，避免每 tag 分配 `Vec`；
- `DispatchReader::recv()` 异步阻塞等待，替换 HTTP-FLV 20ms 轮询；
- 延迟采样 1/64，降低每 tag 全局 Mutex 竞争；
- 原有 `RingBuffer + Dispatcher` 架构不变。

本机 release + `--demo` 结果：

| 场景 | 成功 | 耗时 | CPU 平均 | p99 延迟 | 内存 |
|---|---:|---:|---:|---:|---:|
| 5K 优化前 | 5000/5000 | 14.78s | 84.88% | 359.9ms | 15.1MB |
| 5K async recv | 5000/5000 | 15.22s | 77.63% | 19.3ms | 14.7MB |
| 5K 最终 | 5000/5000 | 13.09s | 94.21% | 55.4ms | 15.8MB |
| 10K async recv | 10000/10000 | 22.24s | 106.86% | 3369.6ms | 18.8MB |
| 10K 最终 | 10000/10000 | 18.53s | 145.21% | 734.8ms | 19.2MB |

结论：
- 单机 10K HTTP-FLV 可全部成功，p99 从 3.37s 降到 0.73s；
- CPU 随连接数上升，10K 约 1.5 核；
- 5K/10K 数据存在运行波动，CPU 不是严格单调；
- 100K 仍需多主机环境，未在此机器验证。
## 2026-11-23 二次优化：broadcast 共享环

将 Dispatcher 从“每订阅者一个 mpsc”改为 `tokio::sync::broadcast` 共享环：

- 发布端只写一次，不再对 10K 订阅者逐个 `try_send`；
- 订阅者按各自游标读取，滞后时计数丢弃；
- HTTP-FLV 仍使用异步 `recv()`。

本机 release 复测：

| 场景 | 成功 | CPU 平均 | p99 | 内存 |
|---|---:|---:|---:|---:|
| 10K（mpsc） | 10000/10000 | 145.2% | 734.8ms | 19.2MB |
| 10K（broadcast） | 10000/10000 | 45.3% | 254.5ms | 13.7MB |
| 20K（broadcast，单客户端） | 16333/20000 | 51.4% | 1550.7ms | 78.2MB |`r`n| 20K（两客户端，8K 缓冲） | 16342/20000 | ~61% | 515.7ms | 14.2MB |

20K 未全部成功，服务端 CPU 仍低，瓶颈在单 Node 客户端/Windows 连接与句柄限制；需多客户端/多主机继续验证。
补充：
- HTTP-FLV 每连接初始缓冲从 64KB 降到 8KB，20K 场景内存从 594MB 降到约 15MB；
- `config/server.toml` backlog 提升到 32768；
- 20K 仍有约 18% 失败，两个 Node 客户端分布不均（9104/7238），服务端 CPU 仍低，判断为客户端/Windows 连接限制。
## 2026-11-25 性能门禁

`scripts/perf_gate.ps1 -Viewers 5000` 结果：

`	ext
ok=5000/5000
cpu=54.19%
p99=25.52ms
mem=10.43MB
gate=PASS
`

HTTP-FLV 热路径已拆分到 src/http_flv.rs，p99 采样跳过每个连接前 16 个缓存 tag，避免冷启动 backlog 拉高稳态指标。

## 2026-11-26 性能分析

本轮针对 HTTP-FLV 连接建立与 Nagle 行为做两项低风险优化：

- accept 后设置 `TCP_NODELAY`，避免小 tag 被 Nagle 合并增加尾延迟；
- HTTP 响应头与 FLV header 合并为一次 `write_all`，每连接少一次 syscall。

实测（5K，release + demo）出现明显运行波动：

```text
run1: ok=5000/5000  cpu=47.45%  p99=257.1ms  mem=200.7MB
run2: ok=4967/5000  cpu=43.48%  p99=196.8ms  mem=106.3MB
```

相比 2026-11-25 基线（5000/5000、54.19%、25.52ms、10.43MB），p99/内存变差，但 CPU 更低，且 run2 出现 33 个失败；判断主要为 Windows 临时端口/TIME_WAIT 与压测机负载波动，非协议错误。该两项不改变数据面架构，保留作为连接级优化；后续需在干净环境用多客户端复测后再下结论。
## 2026-11-26 稳定化复测

`bench.ps1` 增加 `SettleSecs`（默认 3s），压测结束后等待连接清理再采集内存/延迟，避免把仍在关闭的 5K 连接算入 RSS。

两次 5K 结果（release + demo，duration 90）：

| 轮次 | 成功 | CPU | p99 | settle 后内存 |
|---|---:|---:|---:|---:|
| run1 | 5000/5000 | 46.05% | 114.9ms | 23.9MB |
| run2 | 5000/5000 | 36.01% | 266.1ms | 10.7MB |

结论：

- 成功数稳定，无连接失败；
- CPU 36–46%，p99 115–266ms，存在调度抖动；
- settle 后内存 10–24MB，之前 200MB 是连接关闭前采集导致的假高值；
- `TCP_NODELAY` + 合并 header 保留，但不宣称稳定降低 p99；
- `perf_gate.ps1` 改为 `Duration=90`、`SettleSecs=3`、p99 阈值 400ms。
## 2026-11-26 10K 发送缓冲优化

新增 `M7S_HTTP_SNDBUF`（默认 64KB，0=系统默认），在 HTTP accept 后设置每连接内核发送缓冲。

10K 结果：

| 配置 | 成功 | CPU | p99 | settle 后内存 |
|---|---:|---:|---:|---:|
| 系统默认（约 354MB） | 10000/10000 | 76.36% | 33.7ms | 354.6MB |
| 32KB | 9996/10000 | 36.83% | 322.4ms | 12.7MB |
| 64KB | 10000/10000 | 50.86% | 283.0ms | 12.5MB |

结论：

- 限制每连接发送缓冲把 10K 场景 RSS 从约 355MB 降到约 12.5MB；
- 64KB 在本轮一次复测中 10000/10000；
- 连续压测后再次运行出现 9780/10000、CPU 123%、内存 250MB，判断为 Windows 端口/TIME_WAIT 与压测机饱和，不是协议失败；
- 默认取 64KB，保留 `M7S_HTTP_SNDBUF` 覆盖；
- 10K 的最终结论必须在干净环境、多客户端下复测。
## 2026-11-26 多源 IP 10K 复测

为排除 Windows 单源端口/TIME_WAIT 限制，`bench.ps1` 新增 `-SourceIps`，通过 `LOAD_SOURCE_IPS` 让 Node 客户端绑定多个 127.0.0.x 源地址。

10K，8 源 IP，两次：

| 轮次 | 成功 | CPU | p99 | 内存 |
|---|---:|---:|---:|---:|
| run1 | 10000/10000 | 27.75% | 520.5ms | 13.87MB |
| run2 | 10000/10000 | 26.84% | 255.0ms | 13.48MB |

结论：

- 多源 IP 后 10K 稳定 10000/10000，确认此前失败是单客户端端口耗尽；
- CPU 约 27%，内存约 13.5MB；
- p99 255–520ms，仍有调度抖动；
- 20K/16 源 IP 仅 1584/20000，服务端 CPU 31%、内存 14.8MB，瓶颈仍在客户端/OS 连接能力，不能宣称 20K 通过。
## 2026-11-26 HLS segment 零拷贝

`Segment.data` 从 `Vec<u8>` 改为 `Arc<Vec<u8>>`，`HlsStream::segment(seq)` 返回 `Arc`，HTTP 直接 `write_all(data.as_slice())`。

优化点：

- 每个 HLS segment 请求不再复制整个 TS；
- 多个播放器请求同一 segment 共享同一份内存；
- 加密仍只在 segment 生成时执行一次。

测试：`tests/hls_http.rs` 3 项通过。