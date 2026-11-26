# 性能剖析指南

## Windows

本机没有 `cargo-flamegraph`/`perf`，但 Windows 自带 WPR：

```powershell
wpr -start CPU -filemode
# 运行 5K/10K 压测
powershell -File scripts\bench.ps1 -Viewers 5000 -SourceIps 4
wpr -stop profile.etl
```

用 Windows Performance Analyzer 打开 `profile.etl`，查看 CPU Usage (Sampled)
和火焰图。ETL 不应提交到仓库。

## Linux

```bash
cargo install flamegraph
cargo flamegraph --release --bin m7s-rs -- --config config/server.toml --demo
```

或：

```bash
perf record -F 99 -g -- target/release/m7s-rs --config config/server.toml --demo
perf script | stackcollapse-perf.pl | flamegraph.pl > flame.svg
```

## 当前热点分析

| 热点 | 证据 | 处理 |
|---|---|---|
| DispatcherPool 每 tag SipHash | `Stream::push` 曾每 tag 调用 `worker_for(key)` | 已缓存 shard index 到 `Stream::worker_idx` |
| broadcast 唤醒 10K task | 每 tag `tx.send` | 架构限制；帧边界 batch 尚未实现 |
| `Stream::push` 多把锁 | last_ts/cache/configs/metadata | 25fps 下可接受；真实高码率需再测 |
| HLS segment 复制 | 每次请求 clone TS | 已改 `Arc<Vec<u8>>` |
| HTTP 每连接发送缓冲 | 10K 约 355MB RSS | 已加 `M7S_HTTP_SNDBUF=64KB` |

## 已测结果

- 10K HTTP-FLV：10000/10000，CPU ~27%，内存 ~13.5MB；
- HLS：405 rps，p99 109ms；
- RTMP：200/200 发布；
- 帧边界固定 2ms batch 未降低 p99，默认关闭。

## 下一步

1. 生成 WPR/Linux flamegraph 并归档到 `docs/perf/`（只提交图片，不提交 ETL）。
2. 按火焰图确认 `broadcast` 唤醒占比。
3. 若唤醒占主导，设计 Dispatcher batch 消息，而不是 viewer sleep。
