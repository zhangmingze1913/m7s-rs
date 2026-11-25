# HLS 互操作验证

## 已验证

- `tests/hls_http.rs`：真实 HTTP 获取 `index.m3u8` / `seg-0.ts`；
- `tests/fmp4_http.rs`：fMP4 init/segment、LL-HLS blocking/`_HLS_part`；
- TS 以 `0x47` 同步字节开始，PAT/PMT/PES 生成；
- HLS 切片由 AVC 关键帧驱动，SPS/PPS 来自 FLV AVC sequence header；
- AES-128：TS 与 fMP4 `av.m3u8`/`aud.m3u8` 的 `#EXT-X-KEY`、`key.bin`、CBC/PKCS7；
- LL-HLS：`EXT-X-PART`、`PRELOAD-HINT`、`PART-INF`、part 路由；
- **多码率 master**：`/hls/<app>/<name>/master.m3u8` 自动列出
  `<name>` 与 `<name>_<kbps>` 流。

## 多码率命名

```text
/hls/live/main/master.m3u8
  -> ../main/index.m3u8        BANDWIDTH=800000
  -> ../main_1500/index.m3u8   BANDWIDTH=1500000
```

测试：

```text
tests/hls_master.rs::hls_master_lists_variants ... ok
```

变体编码仍由外部 encoder/Transcode 产生；master 只负责聚合与声明。

## fMP4 AES-128\n\n测试：\n\n```text\ntests/fmp4_encrypt.rs::fmp4_aes128 ... ok\n```\n\n覆盖：`av.m3u8` 的 `EXT-X-KEY`、`key.bin` 16 字节、`av-0.m4s` 加密后不含明文 `moof`、CBC 解密恢复 `moof`。\n\n## 播放器命令

```bash
ffplay -fflags nobuffer http://127.0.0.1:8180/hls/live/test/index.m3u8
ffplay http://127.0.0.1:8180/hls/live/test/master.m3u8
vlc http://127.0.0.1:8180/hls/live/test/index.m3u8
```

## 已知限制

- 仅 H.264 + AAC；
- fMP4/TS 片段支持 HTTP `Range: bytes=`（206 Partial Content）；`part-*` 仍为明文；
- B 帧 DTS/PTS 复杂场景未覆盖；
- 真实播放器弱网/ABR 切换未做仪器化验收；
- 变体流需外部转码产生，未内置多码率编码。
