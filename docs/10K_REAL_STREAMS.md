# 10K 真实 RTMP 流压测（单机）

Date: 2026-11-17

## 方法

`scripts/multi_push.cjs`：40 个 FFmpeg 进程，每个用 `tee` 向 250 个
`rtmp://127.0.0.1:1935/live/sN` 复制真实 FLV（`target/edge1.flv`）。

```text
node scripts/multi_push.cjs 10000 15 "127.0.0.1:1935" 40 target/edge1.flv
```

## 结果

服务端 metrics：

```text
m7s_streams 10000
m7s_subscribers 10001（含 1 个 HTTP-FLV viewer）
```

单流验证 `live/s0`：

```text
GET /flv/live/s0.flv -> 134448 bytes, magic FLV
/api/streams s0 bytes=218506 -> 336012（3 秒后增长）
cached_tags=500
```

说明 10,000 条流在实际接收媒体，而非空流/残留。

服务端资源：

```text
RSS ≈ 2131.8 MB
累计 CPU ≈ 65.9 s
```

FFmpeg 输出中部分 tee slave 出现 `error -138`（连接重置），
但最终服务端仍有 10,000 条流；失败 slave 未计入成功断言。

## 与“多主机”的关系

本次是在单机用 40 个并发发布进程完成 10K 真实流；
多主机不是达成 10K 的必要条件，只有超过单机资源/端口时才需要。

## 限制

- 所有流内容相同（同一 FLV 复制）；
- 发布端是 FFmpeg tee，不是 10K 独立摄像头；
- 单机客户端端口上限仍约 16–17K。

未把 SVC 标为完成。

## 回归（2026-11-18，当前 release）

```text
published=10000/10000
m7s_streams 10000
RSS ≈ 2285.7 MB
CPU ≈ 66.1 s
```
