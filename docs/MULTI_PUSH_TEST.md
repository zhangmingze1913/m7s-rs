# 真实多源推流测试

Date: 2026-11-17

## 工具

`scripts/multi_push.cjs`：用 FFmpeg `tee` 把真实 FLV 复制到 N 个 RTMP 输出，
支持多节点、可配置 worker 数。

```text
node scripts/multi_push.cjs <streams> <seconds> <nodes> [workers] [sample.flv]
nodes = "127.0.0.1:1935,127.0.0.1:1936"
```

## 结果

### 单节点 20 流

```text
total=20 workers=2
published=20/20 active_at_check=20 elapsed_s=11.50
```

### 单节点 100 流

```text
total=100 workers=4
published=100/100 active_at_check=100 elapsed_s=11.69
server RSS=28.8 MB（结束后）
```

### 双节点 30 流

origin 1935 + edge 1936：

```text
published=30/30 active_at_check=115
```

（active 包含此前仍在线的 100 条流；30 条新流均匀分布到两个节点。）

## 10K 扩展

脚本本身按 `streams/nodes/workers` 扩展；10K 需要：

```text
node scripts/multi_push.cjs 10000 60 "host1:1935,host2:1935,..." 100
```

限制：
- 单机 FFmpeg 编码/复用开销太大；
- 需要多主机 + 多 worker；
- 本机未执行 10K 真实流，只验证到 100 流。

## SVC

跳过；无外部 SVC 编码器，capability 仍 `svc:false`。
