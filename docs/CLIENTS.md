# 客户端测试

## 1. FFmpeg / OBS（RTMP 推流）

```bash
# 生成测试源
ffmpeg -re -f lavfi -i testsrc=size=640x360:rate=25 \
  -f lavfi -i sine=frequency=1000 \
  -c:v libx264 -preset ultrafast -tune zerolatency \
  -c:a aac -f flv rtmp://127.0.0.1:1935/live/test
```

OBS：

- 服务：自定义
- 服务器：`rtmp://127.0.0.1:1935/live`
- 推流密钥：`test`
- 需要鉴权时：`test?key=YOUR_KEY`

## 2. HTTP-FLV 播放

```bash
ffplay -fflags nobuffer http://127.0.0.1:8180/flv/live/test.flv
vlc http://127.0.0.1:8180/flv/live/test.flv
```

浏览器可用 flv.js：

```html
<script src="https://cdn.jsdelivr.net/npm/flv.js/dist/flv.min.js"></script>
<video id="v" controls></video>
<script>
const p = flvjs.createPlayer({type:'flv', url:'http://127.0.0.1:8180/flv/live/test.flv'});
p.attachMediaElement(document.getElementById('v'));
p.load(); p.play();
</script>
```

跨域：服务端已返回 `Access-Control-Allow-Origin: *`。

## 3. HTTP API 探测

```bash
curl -s http://127.0.0.1:8180/health
curl -s http://127.0.0.1:8180/api/streams
curl -s http://127.0.0.1:8180/cluster/api/status/local
curl -s http://127.0.0.1:8180/cluster/api/sessions
```

Python：

```python
import json, urllib.request
for path in ("/health", "/api/streams", "/cluster/api/sessions"):
    with urllib.request.urlopen("http://127.0.0.1:8180" + path) as r:
        print(path, json.load(r))
```

Node.js：

```js
for (const p of ['/health', '/api/streams', '/cluster/api/sessions']) {
  const r = await fetch('http://127.0.0.1:8180' + p);
  console.log(p, await r.json());
}
```

## 4. RTMP 客户端测试

`tests/rtmp_publish.rs` 实现了一个最小 RTMP 客户端：

- 握手
- AMF0 connect/createStream/publish
- 发送 video tag
- 通过 HTTP-FLV 读回并断言

## 5. 集群客户端测试

```bash
# Origin / Edge 分别启动
cargo run --release -- --config config/cluster-origin.toml
cargo run --release -- --config config/cluster-edge.toml

# 向 Origin 推流
ffmpeg ... rtmp://127.0.0.1:1935/live/test

# 检查会话
curl http://127.0.0.1:8180/cluster/api/sessions
# 从 Edge 播放（触发回源）
ffplay http://127.0.0.1:8181/flv/live/test.flv
```

自动化：`cargo test --test cluster_flv -- --nocapture`。

## 6. 兼容性目标

| 客户端 | 协议 | 当前状态 |
|---|---|---|
| FFmpeg | RTMP/HTTP-FLV | 目标兼容 |
| OBS | RTMP | 目标兼容 |
| VLC | HTTP-FLV | 目标兼容 |
| ffplay | HTTP-FLV | 目标兼容 |
| flv.js | HTTP-FLV | 目标兼容 |
| Python/Node/curl | HTTP API | 已支持 |
| Monibuca Web SDK | HTTP-FLV/API | 需按其 API 适配 |
