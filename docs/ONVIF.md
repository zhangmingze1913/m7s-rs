# P4 ONVIF

## Device Service

```text
POST http://127.0.0.1:8083/onvif/device_service
Content-Type: application/soap+xml
```

Supported SOAP actions:

- `GetDeviceInformation`
- `GetCapabilities`
- `GetStreamUri`

Verified:

```xml
<tds:Manufacturer>m7s-rs</tds:Manufacturer>
<tds:Model>m7s-rs</tds:Model>
<tds:FirmwareVersion>0.1.0</tds:FirmwareVersion>
```

`GetStreamUri` returns `rtsp://127.0.0.1:8554/live/test`.

## Not yet implemented

- WS-Discovery (UDP 3702);
- ONVIF authentication/WS-Security;
- Media/PTZ service operations;
- real camera/device interoperability.

## WS-Discovery

Multicast UDP `239.255.255.250:3702` with `SO_REUSEADDR`.

Verified ProbeMatches:

```xml
<d:Types>dn:NetworkVideoTransmitter</d:Types>
<d:XAddrs>http://198.18.0.1:8083/onvif/device_service</d:XAddrs>
```

Set `M7S_ONVIF_HOST` to override the advertised host.
## Media Profiles

`GetProfiles` on `http://127.0.0.1:8083/onvif/media_service` returns:

```xml
<trt:Profiles token="profile1" fixed="true">
  <tt:Name>main</tt:Name>
  <tt:VideoEncoderConfiguration token="vec1">
    <tt:Encoding>H264</tt:Encoding>
    <tt:Resolution><tt:Width>320</tt:Width><tt:Height>240</tt:Height></tt:Resolution>
  </tt:VideoEncoderConfiguration>
</trt:Profiles>
```
## Authentication

Set `M7S_ONVIF_USER` and `M7S_ONVIF_PASSWORD` to require WS-Security
UsernameToken with `PasswordDigest`.

Verified:

```text
no auth  -> HTTP 401
valid WS-Security digest -> HTTP 200 + GetDeviceInformationResponse
```

## PTZ

`POST /onvif/ptz_service` supports:

- `ContinuousMove` (PanTilt x/y)
- `Stop`
- `GetStatus`

Verified ContinuousMoveResponse and GetStatusResponse are returned; hardware
PTZ is not connected.
## Media2 / Events surface

Added SOAP actions:

- `GetVideoEncoderConfigurations`
- `GetVideoSources`
- `GetEventProperties` (MotionAlarm topic set)

Test `tests::media2_and_events` starts the real ONVIF HTTP server on an
ephemeral port and posts all three requests.

## Extended Device/Media2/PTZ/Events surface（2026-11-21）

新增 SOAP 操作：

| 分类 | 操作 |
|---|---|
| Device | `GetServices`、`GetServiceCapabilities`、`GetScopes`、`GetSystemDateAndTime`、`GetNetworkInterfaces` |
| Media2 | `GetVideoSourceConfigurations`、`GetAudioSources`、`GetAudioEncoderConfigurations`、`GetVideoEncoderConfigurationOptions`、`SetVideoEncoderConfiguration` |
| PTZ | `AbsoluteMove`、`RelativeMove`、`GetConfigurations`、`SetPreset`、`GetPresets`、`GotoPreset`、`RemovePreset` |
| Events | `CreatePullPointSubscription`、`PullMessages`（空通知列表） |

测试：

```text
tests::onvif_extended_surface ... ok
```

说明：这些是协议/服务端实现；真实 ONVIF 摄像头联调、真实云台闭环、事件推送仍需设备。
`GetStreamUri` 目前返回固定 RTSP 地址，接入真实设备时需按 profile/stream 配置映射。

## ONVIF Client（2026-11-22）

`m7s_plugin_onvif::client::OnvifClient`：

```rust
OnvifClient::new("http://cam/onvif/device_service")
    .with_auth("admin", "password")
    .get_device_information().await?;
```

支持：

- `GetDeviceInformation`；
- `GetProfiles`；
- `GetStreamUri`（解析 `<tt:Uri>`）；
- `ContinuousMove` / `Stop`；
- **WS-Discovery 客户端**：
  ```rust
  discover(Duration::from_secs(2)).await?;      // 239.255.255.250:3702
  discover_at(addr, timeout).await?;             // 可测试/定向
  ```
- WS-Security UsernameToken：
  - Base64(SHA1(nonce + Created + password))；
  - 16 字节随机 nonce、RFC3339 Created。

测试：

```text
tests/onvif_client.rs::onvif_client_against_mock ... ok
tests/onvif_client.rs::onvif_discovery_client ... ok
```

## 自动拉流

```text
GET /api/onvif/pull?endpoint=http://cam/onvif/device_service&profile=profile1&stream=live/cam
```

流程：`GetStreamUri` → `rtsp_pull::pull()`。

测试：

```text
tests/onvif_pull.rs::onvif_stream_uri_to_rtsp_pull ... ok
```

真实摄像头仍需：
- 厂商 WS-Discovery/WS-Security 差异；
- 厂商 profile URI/音频/重连差异。
## Events PullPoint 队列（2026-11-22）

- push_event(topic, value) 入队（最多 100 条）；
- PullMessages 每次取走全部事件并返回 NotificationMessage；
- HTTP 注入：

`	ext
GET /api/onvif/event?topic=tns1:VideoSource/MotionAlarm&value=true
`

测试：	ests::onvif_extended_surface（事件 + 空队列二次 Pull）。
