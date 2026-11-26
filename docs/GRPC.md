# gRPC 控制面

## 服务

```text
:m7s.StreamService/ListStreams
:m7s.StreamService/GetStream
:m7s.StreamService/CreateStream
:m7s.StreamService/DeleteStream
:m7s.StreamService/GetStatus
:m7s.StreamService/WatchStreams

:m7s.PluginService/ListPlugins
:m7s.RoomService/CreateRoom
:m7s.RoomService/JoinRoom
:m7s.RoomService/LeaveRoom
:m7s.RoomService/ListRooms
:m7s.RoomService/CloseRoom
```

默认 h2c `0.0.0.0:50051`。

## Token 鉴权（2026-11-21）

设置：

```text
M7S_GRPC_TOKEN=secret
```

请求头：

```text
authorization: Bearer secret
```

未携带/错误返回 `grpc-status: 16`（Unauthenticated），不返回业务 body。

测试：

```text
tests/grpc_auth.rs::grpc_token_auth ... ok
```

## TLS（2026-11-21）

配置：

```toml
[tls]
enabled = true
grpc = true
cert = "tests/fixtures/server.pem"
key = "tests/fixtures/server-key.pem"
```

`main` 使用 `grpc::run_tls(listener, streams, tls_acceptor)`，
在 TLS 上跑 h2 + gRPC。

测试：

```text
tests/grpc_tls.rs::grpc_tls_roundtrip ... ok
```

## 仍开放

- gRPC 流式推送（watch/日志）；
- 与 HTTP API 的全量行为对照；
- 独立 gRPC 端口/证书配置项（当前复用 `[tls]`）。

## 服务端流（2026-11-22）

WatchStreams 每 50ms 推送一个 StreamList frame，共 3 次，然后 grpc-status: 0。
测试：	ests/grpc.rs::grpc_watch_streams。

## mTLS（2026-11-22）

[tls] require_client_cert=true + client_ca 时，gRPC TLS 使用
	ls::load_server_with_client_auth。
测试：	ests/grpc_mtls.rs（客户端证书成功）；无证书拒绝由 	ests/mtls_http.rs 覆盖同一 acceptor。

## HTTP/gRPC Stream CRUD 对照（2026-11-22）

	ests/api_parity.rs 验证 List/Create/Delete 在 gRPC 与 HTTP 间一致；
Room/Plugin/Status 全量对照仍未覆盖。

## 2026-11-26 Room 字段级补充

新增：

```text
:m7s.RoomService/GetRoom
:m7s.RoomService/SendChat
:m7s.RoomService/SendGift
```

- `GetRoom` 返回 `room/members/robots/gifts/seats`；
- `SendChat` 返回 `ok/error/user/text/ts`；
- `SendGift` 返回 `ok/total`；
- 与 HTTP `/api/live/status`、`/api/live/chat/send`、`/api/live/gift` 对应。

测试：`tests/grpc_room.rs::grpc_room_matches_http_surface`。
