# Admin RBAC / 审计 / Web SDK

## JWT 角色

`/api/login` 返回带 `role` 的 JWT：

```json
{"code":0,"token":"...","role":"viewer"}
```

角色来源：

- `M7S_ADMIN_PASSWORD`：admin；
- `data/users.json`：用户记录里的 `role`（`viewer`/`admin`）；
- MySQL users：当前 admin；
- API token：admin。

JWT `/api/login` 本身免鉴权；其他 `/api/*` 需要 Bearer。

## RBAC

仅 admin 可访问：

```text
/api/plugins/toggle
/api/stream/delete
/api/users/*
/api/audit
/api/admin/*
```

其他角色返回 `403 forbidden`。

测试：

```text
tests/rbac_audit.rs::rbac_and_audit
```

## 审计

`src/audit.rs` 追加 JSONL：

```text
M7S_AUDIT_LOG=data/audit.log
```

记录字段：

```text
ts / actor / role / action / target / ok
```

已接入：

- plugin.toggle；
- stream.delete；
- user.add。

查询：

```text
GET /api/audit?limit=100        (admin JSON)
GET /api/audit/export?limit=1000 (admin CSV)
M7S_AUDIT_MAX=10000            (append 后保留最近 N 行)
M7S_AUDIT_ARCHIVE=0            (关闭轮转归档，默认开启)
M7S_AUDIT_ARCHIVE_MAX=100000    (归档文件行数上限，超过后滚动)
M7S_AUDIT_ARCHIVE_LEVELS=3     (归档层数 .1/.2/.3)
M7S_AUDIT_COMPRESS=1           (.1→.2 时 gzip 为 .2.gz)
M7S_AUDIT_ROTATE_HOURS=24      (按时间滚动，0=关闭)

归档测试：`audit::tests::rotates_archive_levels`、`archive_policy_tests`。

append + trim 使用进程内互斥锁 + `<log>.lock` 文件锁，避免同/跨进程并发截断丢行；
锁超时 200ms，陈旧锁 >10s 自动清理。
测试：`tests/audit_concurrency.rs`。
```

## Admin UI

React Admin 显示：

- node / role / actor；
- streams/subscribers/rooms/plugins/db；
- rooms/plugins；
- 最近审计条目。

测试：

```text
node web/admin/test.cjs  -> react_admin_ok
```

## Web SDK 1.1

`web/sdk.js` 新增：

- 自动重连：`retry: {max, delay, factor}`，错误/FLV error/ICE failed 触发；
- `onReconnect({attempt, reason})`；
- 多码率/ABR：`levels: [{url, bitrate}]`、`autoLevel()`、`setLevel(i)`；
- `onLevel` 回调；
- `reconnect()` 手动重连；
- `close()` 清理定时器/PC/FLV。

测试：

```text
node web/sdk_test.cjs -> sdk_ok
```

仍开放：真正 HLS 分片带宽测量、DRM、字幕。
