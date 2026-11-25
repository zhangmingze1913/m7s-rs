# Cluster 实测记录（2026-11-16）

## 拓扑

- origin-1: HTTP 8180 / RTMP 1935 / RTSP 8554
- edge-1:   HTTP 8280 / RTMP 1936 / RTSP 8555
- edge seeds: `127.0.0.1:8180`
- edge 启动时禁用 webrtc/srt/onvif/webtransport/cluster-quic/grpc，避免端口冲突

## 结果

```text
origin /health = 200
edge   /health = 200
RTMP push live/cluster -> origin
GET http://127.0.0.1:8280/flv/live/cluster.flv
edge_bytes=251468 magic=FLV
edge stream: bytes=249943 cached_tags=378 subscribers=1 latency_p99_us=53338
```

`/cluster/api/sessions` 返回 edge-1 与 live/cluster；`/cluster/api/servers` 200。

## 覆盖

✅ 两节点组建
✅ edge HTTP 拉流并本地缓存
✅ session 元数据
✅ 真实 FLV 字节校验

## 未覆盖

❌ 多 edge 压测/一致性
❌ 分片/分区分配
❌ 多级 cluster

## QUIC origin failover E2E (2026-11-21)

`tests/quic_failover.rs`：双 origin（127.0.0.1 / 127.0.0.2 :9443），
edge 通过 QUIC 拉 A；kill A 后自动切到 B，原 viewer TCP 连接继续收到字节。

```text
quic_origin_failover_keeps_viewer ... ok
```

同时验证 `@ping` 心跳：A 存活时 `status=ok`，kill 后 probe 失败。

## 复现

```powershell
# origin
.\target\release\m7s-rs.exe --config config\server.toml --duration 80
# edge
$env:M7S_PLUGINS_DISABLED='webrtc-whep,webrtc-whip,onvif,webtransport,srt,cluster-quic,grpc'
.\target\release\m7s-rs.exe --config config\edge.toml --duration 80
# push
ffmpeg ... -f flv rtmp://127.0.0.1:1935/live/cluster
# fetch
curl http://127.0.0.1:8280/flv/live/cluster.flv
```

## 3-node simulation (2026-11-17)

```text
origin-1: 8180 / 1935 / 8554
edge-1:   8280 / 1936 / 8555
edge-2:   8380 / 1938 / 8557
```

RTMP `live/multi` pushed to origin; both edges pulled independently:

```text
edge1 = 203744 bytes, magic FLV
edge2 = 203744 bytes, magic FLV
```

`/cluster/api/sessions` on edge-1 returns live/multi.

This simulates multi-machine topology on loopback with separate node IDs and
ports; it is not real WAN/multi-host testing.

SVC: skipped, capability stays `svc:false` (no external SVC encoder).
## Cluster origin 故障切换 E2E (2026-11-18)

`tests/cluster_process_failover.rs`：

1. 启动 origin 进程（`--demo`，独立端口）；
2. 启动 edge 进程（seed origin）；
3. 从 edge 拉 `/flv/live/test.flv` 读到数据；
4. `kill` origin 进程；
5. 用同一配置重启 origin；
6. 同一 edge 连接继续读到新字节。

测试通过，验证 edge relay 在 origin 进程重启后自动恢复。

```text
origin_process_restart_edge_recovers ... ok
```
## QUIC relay E2E (2026-11-18)

`tests/quic_relay.rs`：

1. `StreamManager` + `live/quic` 推一帧；
2. 启动 `quic_cluster::run`（UDP 9443）；
3. quinn 客户端用 `tests/fixtures/ca.pem` 连接；
4. 发送 `live/quic\n`；
5. 读取 FLV header + tag，断言 `FLV` 与 tag 数据。

```text
quic_relay_delivers_flv_tags ... ok
```