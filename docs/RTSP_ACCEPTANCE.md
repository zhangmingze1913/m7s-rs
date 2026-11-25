# RTSP A/V Acceptance

## Scope

RTSP H.264 + AAC playback over TCP and UDP, using FFmpeg as the acceptance client.

## Command

Start the server:

```powershell
.\target\release\m7s-rs.exe --config config\server.toml
```

Push a test stream:

```powershell
.\node_modules\ffmpeg-static\ffmpeg.exe -re `
  -f lavfi -i testsrc=size=320x240:rate=25 `
  -f lavfi -i sine -t 10 -g 25 -keyint_min 25 `
  -c:v libx264 -preset ultrafast -tune zerolatency -pix_fmt yuv420p `
  -c:a aac -f flv rtmp://127.0.0.1:1935/live/ff
```

Run acceptance:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\accept_rtsp_av.ps1 `
  -Url rtsp://127.0.0.1:8554/live/ff
```

## Result

```text
== RTSP A/V acceptance: tcp ==
PASS: tcp
== RTSP A/V acceptance: udp ==
PASS: udp
RTSP A/V acceptance passed (FFmpeg, TCP + UDP)
```

The script fails on non-zero FFmpeg exit and runs both audio and video output maps.

## RTP verification

The diagnostic TCP proxy (`examples/rtsp_proxy.rs`) captured a VLC TCP session:

```text
frames=895
chans={0: 577, 1: 1, 2: 316, 3: 1}
gaps=0
video: 577 RTP packets
audio: 316 RTP packets
```

No RTP sequence gaps were observed.

## Known caveat

VLC 3.0.16 / 3.0.21 direct playback of the A/V stream can log:

```text
main decoder error: buffer deadlock prevented
```

VLC through the TCP proxy, and FFmpeg direct TCP/UDP, both pass. The VLC direct
decoder behavior is tracked separately and is not part of the FFmpeg acceptance
result.

## RTSP multicast（2026-11-22）

`SETUP` 支持：

```text
Transport: RTP/AVP;multicast
```

响应：

```text
Transport: RTP/AVP;multicast;destination=239.255.1.x;port=30000-30001;ttl=16;ssrc=12345678
```

- 组地址按连接递增分配；
- RTP socket 使用 `socket2` 设置 `IP_MULTICAST_IF`/TTL/loop；
- 出口网卡可用 `M7S_RTSP_MULTICAST_IF=<ipv4>` 覆盖，默认探测默认路由网卡。

测试：

```text
tests/rtsp_multicast.rs::rtsp_multicast_video ... ok
```

测试覆盖 SETUP/PLAY 协商与组地址/端口校验。实际组播 RTP 接收依赖主机/交换机组播配置；
Windows loopback 组播在部分环境不回流，因此未把收包作为本机强制断言。

RTCP 组播发送仍使用默认网卡。

## RTSP 音视频 backchannel（2026-11-22）

TCP interleaved 支持客户端 channel 0 视频 / channel 2 音频：

```text
$ 2 <len> RTP(MPEG4-GENERIC AU headers)
→ source stream 写入 FLV video/audio frame
``

测试：

```text
tests/rtsp_backchannel.rs::rtsp_backchannel_audio ... ok
tests/rtsp_backchannel_video.rs::rtsp_backchannel_video ... ok
```

## RTSP Digest auth（2026-11-21）

启用：

```text
M7S_RTSP_USER=admin
M7S_RTSP_PASSWORD=secret
```

- OPTIONS 不需要鉴权；
- DESCRIBE/SETUP/PLAY 无 `Authorization` 时返回：
  ```text
  RTSP/1.0 401 Unauthorized
  WWW-Authenticate: Digest realm="m7s", nonce="...", qop="auth"
  ```
- 校验 MD5 HA1/HA2、nonce 有效期、qop/nc/cnonce。

测试：

```text
tests/rtsp_digest.rs::rtsp_digest_auth ... ok
```
