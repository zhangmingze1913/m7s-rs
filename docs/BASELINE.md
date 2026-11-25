# P0 基线

## 代码

| 项 | 值 |
|---|---|
| Rust LOC | ~4,741 |
| 单元测试 | 16 |
| 集成测试 | 7 |
| Release | ~4.76 MB |
| Rust | 1.99.1 |
| 平台 | Windows x64 |

## 功能

- RTMP 基础推/拉
- HTTP-FLV
- HLS（H.264/AAC，部分）
- RTSP（TCP/UDP 基础，RTCP SR/RR，VLC 未验证）
- GB28181 SIP REGISTER/MESSAGE（无 Catalog/INVITE/RTP/PS）
- HTTPS/RTMPS/mTLS
- HTTP-FLV Origin/Edge Relay
- FLV 录制、只读 Admin、API Token、基础 metrics

## 压测

| 并发 | 结果 | 备注 |
|---|---|---|
| 1000 | 1000/1000 | 合成流，Windows |
| 5000 | 5000/5000 | 合成流，backlog 16384 |
| 10000 | 5701/10000 | 未通过 |

未采集：P50/P99 帧延迟、CPU/内存/带宽、72h 浸泡、弱网。

## 复现

```powershell
cargo build --release --offline
.\scripts\bench.ps1 -Viewers 1000 -Seconds 3 -Duration 45
```

输出 JSON：viewers、elapsed、working_set_mb、private_mb、cpu_sec、cpu_pct_avg。

## 结论

P0 只提供**可重复的粗略基线**，不能作为 V6 性能对比。P1 必须先做数据面（RingBuffer/ArcSwap/零拷贝/背压），再做完整协议和集群。
