# m7s-rs 验收矩阵（2026-11-21）

状态说明：`PASS` = 自动化可重复验证；`PARTIAL` = 代码完成但缺真机/外部资源；`BLOCKED` = 需外部资源。

| 领域 | 状态 | 自动化验证 | 未完成 / 阻塞 |
|---|---|---|---|
| RTMP 推流 / HTTP-FLV | PASS | `tests/flv_http.rs`、`tests/rtmp_ingest.rs` | RTMP 复杂 AMF/加密 |
| RTSP TCP/UDP A/V | PASS | `tests/rtsp_play.rs` | — |
| RTSP Digest | PASS | `tests/rtsp_digest.rs` | — |
| RTSP multicast | PASS（协商） | `tests/rtsp_multicast.rs` | 组播 RTP 真机接收 |
| RTSP pull（客户端） | PASS | `tests/rtsp_pull.rs`、`tests/rtsp_pull_reconnect.rs`、`tests/rtsp_pull_rtcp.rs` | 真机 |
| RTSP backchannel | PASS（A/V/TCP） | `tests/rtsp_backchannel.rs`、`tests/rtsp_backchannel_video.rs` | 真机 |
| HLS TS / fMP4 / LL-HLS / AES / master / Range | PASS | `tests/fmp4_http.rs`、`tests/fmp4_encrypt.rs`、`tests/hls_http.rs`、`tests/hls_master.rs` | 真实 ABR/弱网 |
| SRT listener/egress/AES | PASS | `tests/srt_egress.rs`、`crates/m7s-plugin-srt` | 真实 SRT 链路弱网 |
| WHEP / WHIP | PASS | `tests/whep_media.rs`、`tests/whip_media.rs` | 浏览器/硬件编码器联调 |
| TWCC per-packet / loss / jitter | PASS（模拟） | `twcc_est_tests::*`、`tests/whep_media.rs` | 真实弱网仪表 |
| WebTransport datagram/stream/egress | PASS | `tests/webtransport.rs` | 浏览器兼容矩阵 |
| GB28181 SIP / 点播 / 回放 / 录像列表 | PASS（信令） | `crates/m7s-plugin-gb28181` 26 tests | 真机录像内容/PTZ 闭环 |
| ONVIF 服务端 + 客户端 + 自动拉流 | PASS（mock） | `onvif_extended_surface`、`tests/onvif_client.rs`、`tests/onvif_pull.rs` | 真机云台/厂商差异 |
| QUIC cluster + failover + shard allocation/redirect | PASS | `tests/quic_failover.rs`、`tests/cluster_allocate.rs`、`tests/shard_redirect.rs` | 跨主机媒体路由/100K |
| gRPC CRUD + TLS/mTLS + token + WatchStreams + Context | PASS | `tests/grpc*.rs`、`tests/api_parity.rs` | 字段级差异 |
| SQLite 迁移 / MySQL | PASS | `meta_db::schema_migrates*`、`tests/mysql_users.rs` | Postgres/连接池 |
| Room/Live 麦位/禁言/踢人/礼物价值/周期/流水/持久化/限流/机器人/审核/录制/混流 | PASS | `tests/room_http.rs`、live tests | 机器人 DSL |
| Meeting 发布/录制/禁言/举手/踢人 | PASS | `tests/room_http.rs`、`mp4::record` | 多人合成 |
| Admin RBAC / 审计 / CSV / 保留+多级归档/gzip/时间滚动+并发锁 | PASS | `tests/rbac_audit.rs`、`tests/audit_concurrency.rs` | 多租户/配置热更新 |
| Web SDK 重连/ABR（WebRTC+hls.js+FLV） | PASS | `web/sdk_test.cjs` | 弱网真机/DRM/字幕 |
| WASM market / hot reload / 签名/白名单/fuel | PASS | `wasm_plugin` tests、`tests/wasm_http.rs` | 更细粒度隔离 |
| SEI 原生 / Mix RGB / RGB scale | PASS | SEI/Mix/Transcode tests | H.264/AAC 原生编码 |
| 监控 / 告警 / 流事件 / 连接指标 | PASS | `tests/alerts_http.rs`、`tests/events.rs`、`tests/metrics_http.rs` | pprof/tracing/RSS/CPU |
| Edition/授权 | PASS（离线+远程+吊销+激活/连接上限） | `tests/license_*.rs`、`tests/connection_limit.rs`、`tests/protocol_connection_limit.rs` | 计费 |
| 10K viewers（broadcast） | PASS（本机） | `scripts/accept_10k.ps1`、`scripts/accept_viewers.ps1`、`scripts/bench.ps1` | 20K 单客户端受限；100K 需多主机 |
| SVC | BLOCKED | `/api/webrtc/capabilities` 明确 `svc:false` | 需外部 SVC 编码器 |

## 一键验证

```powershell
cd m7s-rs
rustup run 1.94-x86_64-pc-windows-msvc cargo clippy --offline --workspace --all-targets -- -D warnings
rustup run 1.94-x86_64-pc-windows-msvc cargo test --offline --workspace
rustup run 1.94-x86_64-pc-windows-msvc cargo build --release --offline --workspace
node web/sdk_test.cjs
node web/admin/test.cjs
```

本机验收脚本：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/accept_rtsp_av.ps1 -Url rtsp://127.0.0.1:8554/live/ff
powershell -ExecutionPolicy Bypass -File scripts/accept_10k.ps1
powershell -ExecutionPolicy Bypass -File scripts/accept_viewers.ps1
```

## 明确未完成

1. SVC：无外部 SVC 编码器；
2. 100K：单机上限约 13–16K，需多主机分片；
3. 真机：ONVIF/GB28181/SRT/WebRTC 硬件；
4. 完整 LL-HLS 多码率 master；
5. RTSP backchannel/multicast；
6. 原生 H.264/AAC 编码；
7. 在线授权/多租户计费。
