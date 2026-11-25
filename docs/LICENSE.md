# P7 Commercial License

## Token

```text
base64url(payload).base64url(HMAC-SHA256(payload))
```

Payload:

```json
{"edition":"pro","expires":2000000000,"seats":10,"customer":"demo"}
```

Secret: `M7S_LICENSE_SECRET`.

## Runtime

`GET /api/license`:

```json
{"customer":"demo","edition":"pro","expires":2000000000,"reason":"ok","seats":10,"valid":true}
```

Invalid/expired tokens report `edition: community`, `valid: false`.

## Tooling

`examples/license_gen.rs` signs a demo `pro` token.

## Editions（2026-11-21）

| edition | connections | premium protocols | QUIC cluster |
|---|---|---|---|
| free / community | 50 | 否 | 否 |
| pro | 1000 | 是 | 否 |
| enterprise | 无限 | 是 | 是 |

`premium_allowed()`：pro/enterprise 且未过期；
`seats_available(used)`：pro/enterprise 且 `used < seats`，无效/过期 token 为 false；
`cluster_allowed()`：enterprise。

`main` 中 QUIC cluster 已改用 `cluster_allowed()`，避免 pro 误开集群。

## Not yet implemented

- online activation/revocation；
- 连接数硬限制（当前只报告 `connection_limit`）；
- 多租户隔离/billing。


## Enforcement

Set `M7S_ENFORCE_LICENSE=1` to gate premium plugins:

- webrtc-whep / webrtc-whip
- srt
- webtransport
- onvif
- cluster-quic

Without a valid `pro` token those listeners are not started.
`/api/license` reports `enforced` and `seats_used` / `within_seats`.

Verified without a token:

```text
8081=False 8082=False 8083=False
{"edition":"community","enforced":true,"reason":"no license token"}
```
## Seat limit

When `M7S_ENFORCE_LICENSE=1` and a `pro` token has `seats: 0`, new RTMP
publishes are rejected:

```text
ffmpeg ... rtmp://127.0.0.1:1935/live/seat
Error opening output ... -138
GET /api/streams -> {"streams":[]}
```

Other ingress paths (SRT, WebTransport, WHIP, GB28181, cluster relay) return
early when `try_get_or_create` reports the seat limit.

## Tests

```text
license::edition_tests::official_limits
tests/license_enforcement.rs::editions_and_seats
```
## ���߼����ļ���2026-11-22��

```text
M7S_LICENSE_FILE=data/license.token
```

- ���ʱ���� `M7S_LICENSE`�������ȡ�ļ���
- `GET /api/license/activate?token=...`��admin��У��ǩ����д�룻
- `GET /api/license/revoke` ɾ���ļ����� env��
- ���ԣ�`tests/license_activate.rs`��

����Զ�̼���/��������

## Զ�̼��2026-11-22���ܿأ�

```text
GET /api/license/activate-remote?server=http://license-host/activate&token=...
```

- `license::activate_remote` �� HTTP POST �ύ token��
- ����� 200 + ��ѡ�� token��
- ������ǩ��д `M7S_LICENSE_FILE`��
- ���ԣ�`tests/license_remote.rs`��mock server����

����������Ȩ���񡢵����б���Ʒѡ�

## 内置授权服务（2026-11-22）

`	ext
cargo run --bin license-server
M7S_LICENSE_BIND=0.0.0.0:8090
`

- POST /activate 验签返回 token；
- POST /revoke 加入进程内吊销表；
- 吊销后 activate 返回 403；
- `M7S_LICENSE_MAX_ACTIVATIONS=N` 限制成功激活次数，超出返回 429；
- 吊销表持久化到 `M7S_LICENSE_REVOKED`（默认 `data/license-revoked.json`），启动时加载。
测试：	ests/license_server.rs。

## 媒体连接硬限制（2026-11-22）

M7S_MAX_CONNECTIONS=N 在 HTTP/HTTPS/RTMP/RTMPS/RTSP/gRPC/QUIC/WebTransport/WHEP/WHIP/ONVIF accept 与 SRT 会话处限制并发连接；
超限直接断开。
测试：	ests/connection_limit.rs。
