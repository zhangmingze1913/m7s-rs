# 测试规范

## 1. 分层

| 层级 | 范围 | 命令 |
|---|---|---|
| 单元 | AMF0/chunk/FLV/Stream/Config | `cargo test --lib` |
| 集成 | RTMP→HTTP-FLV、Origin→Edge | `cargo test --test ...` |
| 冒烟 | 进程启动/端口/退出 | `cargo run --release -- --duration 3` |
| 静态 | fmt/clippy | `cargo fmt --check` / `cargo clippy` |

## 2. 单元测试矩阵

| ID | 模块 | 场景 | 断言 |
|---|---|---|---|
| U-AMF-01 | amf0 | object roundtrip | 字段/值一致 |
| U-CHUNK-01 | chunk | 400B 消息、128B chunk | 分片重组后 payload 一致 |
| U-FLV-01 | flv | tag 编解码 | 类型/时间戳/data 一致 |
| U-STREAM-01 | stream | GOP 缓存 2 条、广播、订阅计数 | 缓存裁剪/消息/计数 |
| U-HLS-01 | hls | H.264 配置 + 两个关键帧 | 生成 m3u8 与 TS，首字节 0x47 |
| U-REC-01 | record | 推流 tag | 落盘 FLV 头与 tag |
| U-CFG-01 | config | 默认配置 | 校验通过 |
| U-CFG-02 | config | 非法 bind | 校验失败 |

## 3. 集成测试矩阵

| ID | 文件 | 链路 | 断言 |
|---|---|---|---|
| I-RTMP-01 | `rtmp_publish.rs` | RTMP 握手→connect→createStream→publish→video tag→HTTP-FLV | 流存在；FLV header 后有 video tag |
| I-CLUSTER-01 | `cluster_flv.rs` | Origin 有流→Edge 播放→查询 sessions→HTTP-FLV Relay | Edge 返回 FLV header + tag |

## 4. 失败路径与边界

已覆盖：

- 非法配置（bind）。
- FLV 头校验、chunk 分片。
- Edge 无 Origin 时返回 404（逻辑路径）。

待覆盖：

- RTMP 扩展时间戳（0xffffff）。
- AMF0 非法 marker/截断。
- 推流鉴权失败。
- 发布者断线重连、替换旧发布者。
- 慢订阅者 lag/丢帧。
- Origin 下线后的 Relay 重试/切换。
- 并发 100/1k 推流与 10k 播放。

## 5. 集群测试步骤

1. 启动 Origin（`config/cluster-origin.toml`）。
2. 启动 Edge（`config/cluster-edge.toml`）。
3. 向 Origin 推流。
4. 请求 `GET http://edge:8181/flv/live/test.flv`。
5. 验证 Edge 的 `/cluster/api/relay/sessions` 出现 Relay。
6. 停止 Origin，确认 Edge 停止输出/清理 Relay（当前实现为连接断开，不做自动切换）。
7. 重启 Origin 并重新请求，验证可重新回源。

## 5. HLS / 录制 / Admin 测试

```bash
# 推流后
ffplay http://127.0.0.1:8180/hls/live/test/index.m3u8
curl -s http://127.0.0.1:8180/admin | head
curl -s http://127.0.0.1:8180/metrics
ls data/record/live/
```

验收：

- m3u8 含 `#EXTM3U`、`#EXTINF`、`seg-*.ts`；
- TS 以 `0x47` 同步字节开始；
- 录制文件以 `FLV` 开头且持续增长；
- 配置 `api_token` 后，未授权 `/api/streams` 返回 401。

## 6. 通过标准

- 全部单元/集成测试通过；
- Clippy `-D warnings` 无告警；
- 真实 RTMP 推流可在 HTTP-FLV 拉流；
- Edge 无流时能从 Origin 回源；
- 发布/播放/Relay 资源在断开后释放（订阅计数归零）。
