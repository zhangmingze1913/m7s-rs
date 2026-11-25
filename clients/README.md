# 多语言客户端探测

这些脚本只依赖语言标准库：

```bash
python clients/http_probe.py http://127.0.0.1:8180
node clients/http_probe.mjs http://127.0.0.1:8180
```

RTMP 推流/HTTP-FLV 播放请使用 FFmpeg、OBS、VLC、ffplay 或 flv.js，命令见
[`../docs/CLIENTS.md`](../docs/CLIENTS.md)。
