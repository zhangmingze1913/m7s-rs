# 代码质量与注释规范

## 分层

```text
crates/m7s-*        协议/媒体/设备插件，禁止依赖根 crate
src/stream.rs       流注册、GOP cache、延迟
src/dispatcher.rs   每流 shared-broadcast 分发
src/http_flv.rs     HTTP-FLV 热路径（每帧/每连接）
src/hls.rs          HLS segmenter + playlist
src/http.rs         控制面路由、认证、SSE
src/main.rs         装配与监听
```

## 注释策略

本仓库不采用“每个语法行都写注释”的机械方式，因为会产生大量无信息噪声。
要求：

- 模块顶部说明职责和边界；
- 公开 API 说明输入/输出/失败条件；
- 热路径按性能阶段注释：分配、复制、唤醒、背压、释放；
- 非直观的锁顺序、缓存时机、协议字段必须有注释；
- 不使用注释重复代码字面含义。

已逐段注释的重点文件：

```text
src/http_flv.rs
src/dispatcher.rs
src/stream.rs
src/hls.rs
src/wasm_plugin.rs
src/grpc.rs
```

## 已知重构项

```text
src/http.rs 仍约 1580 行，可继续拆分为 http_live/http_admin/http_obs
帧边界批量需要 Dispatcher batch 语义，不能用 viewer sleep 模拟
```

## 性能基线

见 `docs/LOAD_TEST.md`：5K/10K HTTP-FLV、RTMP ingest、HLS segmenter。
