# P5 QUIC Cluster

## Origin relay

- QUIC listener UDP `0.0.0.0:9443`;
- TLS: `tests/fixtures/server.pem`, `tests/fixtures/server-key.pem`;
- edge opens a bidirectional stream and writes `app/name\n`;
- origin replies with FLV header and framed tags:
  ```text
  type u8 | timestamp u32 | len u32 | data
  ```

## Verified

RTMP publish `live/test`, then `examples/quic_edge.rs`:

```text
QUIC_EDGE tags=50 bytes=40927
```

This proves:

- QUIC handshake;
- bidirectional stream request;
- origin stream subscription;
- FLV tag transfer over QUIC.

## Edge transport

QUIC is the primary relay transport. `Cluster::ensure_relay` tries
`origin_host:9443` first and only falls back to HTTP-FLV when the QUIC
handshake/stream fails. `M7S_QUIC_BIND` changes the origin bind address
(default `0.0.0.0:9443`).

Manual edge test:

```text
edge HTTP-FLV: 366 cached tags
ffmpeg -i http://127.0.0.1:8181/flv/live/test.flv -t 2 -f null -
exit 0
```
## Multi-origin selection

`ensure_relay` collects every matching origin from all seeds (deduplicated)
and tries them in order until one relay delivers. If the active origin dies,
the same local `Stream` is kept and the relay rotates to the next origin
mid-stream; viewers keep their edge connection.
## Reconnect / failover

When a relay ends, `ensure_relay` retries the next origin after 500 ms and
keeps cycling while the local stream still has subscribers. This gives
automatic reconnection after an origin disconnect and origin failover.
## QUIC control plane

Request `@sessions\n` on a QUIC bidirectional stream; origin returns:

```text
u32 big-endian length | JSON {"server_id":"quic-origin","sessions":[...]}
```

Verified:

```text
QUIC_CONTROL server="quic-origin" sessions=0
```
## QUIC control-plane discovery

`Cluster::find_origins` queries each seed at `host:9443` with `@sessions`
before falling back to the HTTP control API.

Verified with origin/edge configs:

```text
edge: cluster relay live/test <- 1 candidates
edge: cluster relay pulling http://127.0.0.1:8180/flv/live/test.flv
edge /api/streams: bytes=559363 cached_tags=500 subscribers=1
```

The candidate came from the QUIC control plane, so discovery worked without
using the HTTP session API.
## Reconnect continuity

The relay retry loop keeps the local `Stream` and subscriber set alive while
cycling origins every 500 ms. `Stream::push` normalizes timestamps that jump
backwards by more than one second (origin restart), so the edge timeline keeps
advancing instead of resetting to 0.

Unit test:

```text
timestamp_reset_is_normalized ... ok
```

This provides automatic reconnect and monotonic playback across an origin
restart.

## Heartbeat

The origin answers `@ping\n` on a QUIC bidirectional stream:

```text
u32 big-endian length | JSON {"server_id":"quic-origin","status":"ok","streams":N,"ts":...}
```

`m7s_plugin_cluster::quic_ping(origin)` exposes this probe to the edge/tests.
The failover test calls it before and after killing an origin.

## Origin failover E2E

`tests/quic_failover.rs`:

1. Origin A on `127.0.0.1:9443`, origin B on `127.0.0.2:9443`;
2. Edge seeds both; viewer opens `/flv/live/qfail.flv` on the edge;
3. Edge relays origin A over QUIC; viewer receives FLV bytes;
4. Kill origin A; `quic_ping(A)` fails;
5. Edge rotates to origin B and continues pushing tags;
6. The same viewer TCP connection keeps receiving bytes (no EOF).

```text
quic_origin_failover_keeps_viewer ... ok
```

## Still open

- QUIC datagram media (current media relay is a QUIC stream);
- shard/partition assignment across multiple origins;
- multi-level cluster and WAN fault injection.

## åˆ†ç‰‡åˆ†é…ï¼ˆ2026-11-22ï¼‰

/cluster/api/allocate/play?app=live&stream=name ä½¿ç”¨ FNV-1a åœ¨
self + seeds ä¸Šåšç¡®å®šæ€§åˆ†ç‰‡ï¼Œè¿”å›ž server_id/redirect/addressã€‚
æµ‹è¯•ï¼š	ests/cluster_allocate.rsã€‚

æ³¨æ„ï¼šç›®å‰åªåšåˆ†é…/é‡å®šå‘å†³ç­–ï¼Œè·¨ä¸»æœºåª’ä½“æ‹‰æµä¸ŽåŽ‹æµ‹ä»éœ€éƒ¨ç½²ã€‚

### HTTP ·ÖÆ¬ÖØ¶¨Ïò

```toml
[cluster]
shard_redirect = true
seeds = ["10.0.0.2:8180", "10.0.0.3:8180"]
```

`GET /flv/...` / `GET /hls/.../index.m3u8` ÔÚÁ÷²»ÔÚ±¾½ÚµãÊ±·µ»Ø
`307 Temporary Redirect`£¬Location Ö¸Ïò·ÖÆ¬ owner¡£
²âÊÔ£º`tests/shard_redirect.rs`¡£
