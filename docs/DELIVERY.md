# 交付与验收手册

本文档描述如何构建、配置、启动、验收、升级和回滚 m7s-rs。所有命令均在
workspace 根目录执行。

## 1. 交付物

| 产物 | 路径 |
|---|---|
| Server 二进制 | `target/release/m7s-rs.exe` |
| 授权服务（可选） | `target/release/license-server.exe` |
| 默认配置 | `config/server.toml` |
| Web SDK / Player / Admin | `web/`、`web/admin/dist` |
| 验收文档 | `docs/` |
| 压测脚本 | `scripts/bench.ps1`、`scripts/load_test.cjs` |
| 性能门禁 | `scripts/perf_gate.ps1` |

## 2. 构建

工具链：

```powershell
rustup run 1.94-x86_64-pc-windows-msvc cargo build --release --offline --workspace
```

产物：

```text
target/release/m7s-rs.exe
target/release/license-server.exe
```

## 3. 配置

最小配置见 `config/server.toml`。生产至少确认：

```toml
[server]
http_bind = "0.0.0.0:8180"
rtmp_bind = "0.0.0.0:1935"
backlog = 32768

[auth]
api_token = "change-me"
publish_key = "change-me"

[tls]
enabled = true
cert = "config/cert.pem"
key = "config/key.pem"
```

环境变量（按需）：

```text
M7S_MAX_CONNECTIONS        全局协议连接上限，0=不限
M7S_META_DB                SQLite 元数据库路径
M7S_AUDIT_LOG              审计 JSONL
M7S_AUDIT_MAX              审计保留行数
M7S_AUDIT_ARCHIVE_LEVELS   审计归档层数
M7S_AUDIT_COMPRESS         归档 gzip
M7S_AUDIT_ROTATE_HOURS     按时间滚动
M7S_GIFT_FILE              礼物状态 JSON
M7S_GIFT_EVENT_LOG         礼物流水 JSONL
M7S_LICENSE_FILE           离线授权文件
M7S_LICENSE_SECRET         离线 token HMAC 密钥
```

## 4. 启动

开发/演示：

```powershell
.\target\release\m7s-rs.exe --config config\server.toml --demo --duration 60
```

生产：

```powershell
.\target\release\m7s-rs.exe --config config\server.toml
```

端口：

```text
8180 HTTP / Web / HLS / HTTP-FLV
1935 RTMP
8554 RTSP
8081 WHEP
8082 WHIP
8083 ONVIF
9000 SRT
4443 WebTransport
50051 gRPC
9443 QUIC cluster
8090 可选授权服务
```

## 5. 验收命令

```powershell
rustup run 1.94-x86_64-pc-windows-msvc cargo clippy --offline --workspace --all-targets -- -D warnings
rustup run 1.94-x86_64-pc-windows-msvc cargo test --offline --workspace
rustup run 1.94-x86_64-pc-windows-msvc cargo build --release --offline --workspace
node web/sdk_test.cjs
node web/admin/test.cjs
```

进程级故障切换测试默认标记 `#[ignore]`，需单独串行执行：

```powershell
rustup run 1.94-x86_64-pc-windows-msvc cargo test --offline -p m7s-rs --test cluster_process_failover -- --ignored
``

性能门禁（默认 5K viewers）：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\perf_gate.ps1 -Viewers 5000
```

2026-11-25 基线：

```text
ok=5000/5000
cpu=54.19%
p99=25.52ms
mem=10.43MB
gate=PASS
```

10K 结果与限制见 `docs/LOAD_TEST.md`。

## 6. 验收文件

| 项目 | 证据 |
|---|---|
| 协议 | `docs/ACCEPTANCE_MATRIX.md` |
| V6 差异 | `docs/V6_DIFF_2026-11-18.md` |
| 性能 | `docs/LOAD_TEST.md`、`scripts/bench.ps1` |
| gRPC | `docs/GRPC.md` |
| HLS | `docs/HLS_INTEROP.md` |
| RTSP | `docs/RTSP_PULL.md`、`docs/RTSP_ACCEPTANCE.md` |
| ONVIF/GB28181 | `docs/ONVIF.md`、`docs/GB28181.md` |
| 授权 | `docs/LICENSE.md` |
| Admin/审计 | `docs/RBAC_AUDIT.md` |

## 7. 数据文件与备份

```text
data/meta.db            元数据/KV
data/audit.log          审计
data/audit.log.1..N     审计归档
data/gifts.json         礼物状态
data/gift-events.jsonl  礼物流水
data/mix/               混流输出
record/                 录像
```

备份时至少包含 `data/`、`record/`、`config/`。审计归档如果是 `.gz` 需一并备份。

## 8. 升级

1. 备份 `config/`、`data/`、`record/`；
2. 替换 `m7s-rs.exe`；
3. 启动新版本，观察：
   ```text
   /health
   /metrics
   /api/streams
   ```
4. 执行 `scripts/perf_gate.ps1 -Viewers 5000`；
5. 失败则回滚到上一二进制与配置。

SQLite 迁移在启动时自动执行，只前进不自动回退；回滚前保留旧库副本。

## 9. 回滚

```powershell
Stop-Process -Name m7s-rs -ErrorAction SilentlyContinue
Copy-Item backups\m7s-rs.exe target\release\m7s-rs.exe -Force
Copy-Item backups\data data -Recurse -Force
```

回滚后检查 `docs` 对应版本，避免新配置字段被旧版本忽略。

## 10. 已知边界

- 单机已验证 10K HTTP-FLV；20K 受单客户端/Windows 连接限制；100K 需多主机。
- SVC 无真实编码器。
- ONVIF/GB28181/SRT/WebRTC 未完成真机验收。
- 原生 H.264/AAC 编码仍依赖 FFmpeg。
- 授权计费、多租户未实现。

## 11. 交付检查清单

- [ ] clippy `-D warnings`
- [ ] workspace tests
- [ ] release 构建
- [ ] Web SDK/Admin Node 测试
- [ ] `perf_gate.ps1` PASS
- [ ] `/health`、`/metrics` 正常
- [ ] 配置密钥已替换
- [ ] 备份 `config/data/record`
- [ ] 验收矩阵与 V6 差异文档已同步

## 12. GitHub 发布前检查

本目录尚未初始化为 git 仓库；发布前需要：

```powershell
git init
git add .
git status
git commit -m "m7s-rs v0.1.0"
git remote add origin https://github.com/<org>/<repo>.git
git push -u origin main
```

必须确认：

- `tools/chrome/`、`tools/vlc/`、`tools/vlc321/` 不进入提交（`.gitignore` 已覆盖，约 792MB 第三方二进制）；
- `target/`、`data/`、`record/`、`chrome-*`、`bench*.out`、`load-*.out` 不进入提交；
- 生产证书/私钥不进入提交；`tests/fixtures/*-key.pem` 是测试专用，已在 `tests/fixtures/README.md` 说明；
- CI 使用 `.github/workflows/ci.yml`（Windows + clippy/test/release/Node smoke）；
- 版本号、CHANGELOG、README 状态、DELIVERY 基线一致；
- 打 tag 前运行 `scripts/perf_gate.ps1 -Viewers 5000`，并将 `bench-last.json` 作为构建产物保存而非提交。
## 13. 配置热更新

`GET /api/config/reload`（admin）重新读取启动时的 config 路径，并应用 HLS 配置（enabled/segment_secs/max_segments/encrypt）。监听地址、TLS 证书、RTMP/RTSP 端口仍需重启。测试：`tests/config_reload.rs`。

## 14. 性能诊断

- GET /debug/pprof：streams/subscribers/dropped/parallelism 快照；
- M7S_TRACING=1：启用 tracing-subscriber，HTTP-FLV 连接带 span；
- 未实现 CPU/heap flamegraph；需要时接入 pprof crate。
