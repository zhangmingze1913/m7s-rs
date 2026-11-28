# Docker / Kubernetes 部署与压测

## Docker

```bash
docker build -t m7s-rs:local .
docker run --rm -p 8180:8180 -p 1935:1935 \
  -v "$PWD/data:/data" m7s-rs:local
curl -fsS http://127.0.0.1:8180/health
```

Compose：

```bash
docker compose up -d --build
docker compose ps
curl -fsS http://127.0.0.1:8180/health
docker compose down
```

镜像内已安装 `ffmpeg`、`curl`、CA 证书，非 root 用户 `10001`。

## Kubernetes

```bash
docker build -t m7s-rs:local .
kubectl apply -f deploy/k8s/m7s.yaml
kubectl rollout status deploy/m7s
kubectl port-forward svc/m7s 8180:8180
curl -fsS http://127.0.0.1:8180/health
```

`deploy/k8s/m7s.yaml` 包含 ConfigMap、PVC、Deployment、Service。

## 容器内压测

HTTP-FLV 10K：

```bash
# 宿主机
powershell -File scripts/bench.ps1 -Viewers 10000 -Seconds 8 -SourceIps 8
```

RTMP：

```bash
docker run --rm --network host m7s-rs:local --config /app/config/server.toml
# 宿主机另一终端
powershell -File scripts/rtmp_bench.ps1 -Streams 100
```

HLS：

```bash
powershell -File scripts/hls_bench.ps1 -Seconds 10 -Concurrency 20
```

## 注意

- Docker Desktop/WSL 不可用时无法本机验证容器；CI 的 `docker.yml` 会在 Linux runner 构建镜像。
- K8s 清单用 `kubectl apply --dry-run=client` 校验，实际集群需镜像仓库。
- 数据目录 `/data` 必须持久化；生产替换 `config/server.toml` 的密钥。

## 2026-11-28 实测

- `docker build`（legacy builder，rust:1.100）：成功生成 `m7s-rs:local`
- `docker run` + `/health`：成功
- Docker Desktop 端口映射下 1000 viewers：`ok=82/1000`
- 5000 viewers：`ok=165/5000`
- 容器 CPU 约 4.5%，内存约 9MB

结论：容器本身启动正常；Docker Desktop Windows 端口代理是高并发瓶颈，不能作为 10K 服务端性能结论。需 Linux host network / 直接容器网络或 K8s pod-to-pod 压测。

kind 集群 `mq-rust-k8s` 当前 control-plane/worker2 连接不稳定，`kind load` 报 worker2 容器未运行，未能完成 K8s 部署。