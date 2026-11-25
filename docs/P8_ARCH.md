# P8 Architecture: Workspace Split (start)

## Layout

```text
Cargo.toml                 # root package + [workspace]
crates/m7s-codec/          # foundation crate, zero internal deps
  src/lib.rs               # FlvTag, MediaFrame, TrackKind
src/flv.rs                 # re-exports m7s_codec::FlvTag, FlvReader
src/ring.rs                # lock-free SPMC ring
src/stream.rs              # Stream/StreamManager
```

## m7s-codec

- `FlvTag` (new/from_rtmp/to_bytes);
- `MediaFrame` / `TrackKind` / keyframe detection;
- zero dependency on the engine.

Root keeps `pub use m7s_codec::FlvTag` so existing `crate::flv::FlvTag`
imports keep working without touching every protocol.

## Tests

```text
m7s-codec: 2 tests
m7s-rs:   36 tests
workspace integration tests: 7
```

## Next

- move codec config (SPS/PPS/AAC) into `m7s-codec`;
- add `m7s-sdk` (plugin trait, EngineContext);
- add `config-framework` crate;
- migrate protocols to plugin crates.

## m7s-sdk

`crates/m7s-sdk`:

- `Plugin` trait;
- `PluginSpec`;
- `Registry` with dedupe + enable/disable.

`src/plugins.rs` now builds the 26-entry `/api/plugins` list through the SDK
registry instead of a standalone JSON mapping.

Test: `plugins::tests::registry_has_26`.
## m7s-config

`crates/m7s-config`:

- `load<T: DeserializeOwned>(path)`;
- `from_str<T>`;
- unified read/parse errors.

`src/config.rs::Config::load` now delegates to `m7s_config::load`.

## FLV moved to codec

`FlvReader` and `flv_header` moved into `m7s-codec`; `src/flv.rs` is a
re-export shim plus roundtrip test.
## V6 API surface

`/v6/api/*` now routes to the same AppFacade handlers as `/api/*`:

```text
GET /v6/api/streams  -> {"code":0,"data":{"streams":[...]}}
GET /v6/api/plugins  -> {"count":26,...}
GET /v6/api/license
GET /v6/api/health
```

Auth applies to both `/api` and `/v6/api`.

Verified with curl.
## First protocol plugin crate: m7s-plugin-rtmp

Moved pure RTMP codec (AMF0 + chunk stream) into
`crates/m7s-plugin-rtmp`; the root `src/rtmp/mod.rs` re-exports:

```rust
pub use m7s_plugin_rtmp::{amf0, chunk};
```

The RTMP server/session code remains in the engine until `EngineContext` is
available to plugins. Workspace now has:

```text
m7s-codec
m7s-sdk
m7s-config
m7s-plugin-rtmp
m7s-rs (engine)
```

TS muxer moved into `m7s-codec` (`m7s_codec::TsMuxer`).
## Snap plugin (official V6 media plugin)

`crates/m7s-plugin-snap`:

- `capture(input, output)` runs FFmpeg (`-frames:v 1`, 5s cap, 10s kill);
- JPEG validation (`FF D8 FF`);
- engine endpoint `GET /api/snapshot?stream=app/name`.

Verified:

```text
GET /api/snapshot?stream=live/snap
{"code":0,"path":"data/snapshot/live_snap.jpg"}
size=9370 magic=FFD8FF
```

`/api/plugins` now lists `snap` (replacing the non-V6 `admin` entry).
## Official missing-plugin crates (batch)

Created and tested:

```text
m7s-plugin-mp4        FFmpeg MP4 record + ftyp check
m7s-plugin-sei        SEI NAL detection/extraction
m7s-plugin-mix        RGB frame compositing
m7s-plugin-transcode  FFmpeg transcode args/run
m7s-plugin-crypto     HMAC-SHA256 sign/verify
m7s-plugin-debug      runtime snapshot
m7s-plugin-logrotate  size-based rotation
m7s-plugin-report     atomic counters
m7s-plugin-crontab    interval scheduler
m7s-plugin-test       self-test registry
m7s-plugin-homekit    pairing code
m7s-plugin-v4l2       fourcc helpers
m7s-plugin-alsa       PCM validation
m7s-plugin-live       live room state machine
m7s-plugin-meeting    meeting room state
```

All compile under `cargo clippy --workspace --all-targets -D warnings`;
each has at least one unit test where meaningful.

Protocol crates still in engine: RTSP/FLV/HLS/WebRTC/SRT/WebTransport/
GB28181/ONVIF/Cluster. These need `EngineContext` before extraction.
## MP4 plugin end-to-end

`GET /api/record/mp4?stream=live/x&secs=N` pulls the stream over RTSP and
remuxes with `m7s-plugin-mp4`.

Verified:

```text
{"code":0,"path":"data/mp4/live_mp4.mp4"}
size=43554  ftyp=ftyp
RTSP OPTIONS -> DESCRIBE -> SETUP -> PLAY
```
## EngineContext / PluginHost (SDK)

`m7s-sdk` now defines:

- `EngineContext { node_id, http_base, rtsp_base }`;
- `PluginRuntime { name, start, stop }`;
- `PluginHost { start, stop, is_running, running }`.

Test: `host_lifecycle`.

## FLV plugin crate

`m7s-plugin-flv` re-exports `FlvTag`, `FlvReader`, `flv_header` from
`m7s-codec`; root `src/flv.rs` now re-exports the plugin crate.

## ONVIF plugin crate

`src/onvif.rs` moved to `crates/m7s-plugin-onvif` with:

```rust
pub async fn run(listener: TcpListener)
pub async fn discovery()
```

The engine no longer passes `StreamManager` into ONVIF; main calls
`onvif::run(listener)`.
## Transcode plugin end-to-end

`GET /api/transcode?stream=live/x&secs=N&width=W` pulls RTSP and writes FLV
through `m7s-plugin-transcode` (`libx264` + `aac`, `-t` bounded).

Verified:

```text
{"code":0,"path":"data/transcode/live_tc.flv"}
size=444 magic=FLV
ffmpeg -i ... -f null - exit 0
RTSP OPTIONS/DESCRIBE/SETUP(video+audio)/PLAY
```
## SRT plugin crate

`crates/m7s-plugin-srt` now owns the SRT listener + TS demux.
It depends only on `m7s-sdk` traits:

```rust
TagSink { push_tag(type, ts, data); leave() }
StreamApi { stream(app, name) -> Option<Arc<dyn TagSink>> }
```

Engine adapters in `src/stream.rs`:

- `Stream: TagSink`
- `StreamManager: StreamApi`

Verified end-to-end:

```text
ffmpeg -f mpegts srt://127.0.0.1:9000?streamid=live/srt  exit 0
GET /api/streams -> live/srt bytes=165587 cached_tags=251
```

`/webtransport` reuses `m7s_plugin_srt::TsDemux`.
## WebTransport plugin crate

`src/webtransport.rs` moved to `crates/m7s-plugin-webtransport`:

```rust
pub async fn run(addr: &str, streams: Arc<dyn StreamApi>)
```

It reuses `m7s_plugin_srt::TsDemux` and the SDK `StreamApi` trait; the root
file is a re-export.

Verified:

```text
SENT 116372 bytes
{"key":"live/wt","bytes":81091,"cached_tags":125}
```
## TagStream pull API

SDK now provides pull-side access for egress plugins:

```rust
pub struct Tag { tag_type, timestamp, data }
pub trait TagReader { fn try_next(&mut self) -> Option<Tag>; }
pub trait TagStream: TagSink { fn tag_reader(&self) -> Box<dyn TagReader>; }
```

`Stream` implements `TagStream` over its lock-free ring; subscriber count is
incremented and released with `TagSink::leave`.

Test: `stream::tests::tag_stream_reader`.

This unblocks HLS/RTSP/WebRTC/Cluster egress plugin migration.
## HLS plugin crate (playlist)

`crates/m7s-plugin-hls` provides:

```rust
PlaylistSegment { seq, duration }
playlist(segments, target_secs) -> String
```

Engine `HlsStream::playlist` now delegates to the plugin; HLS segmenter
remains in the engine until it consumes `TagStream`.

Verified by existing HLS integration test plus plugin unit test.
## HLS segmenter migrated to TagStream

`m7s-plugin-hls` now owns `Segmenter` (FLV tag -> MPEG-TS segments) and
`adts_frame`. Engine `hls::segment_loop`:

```rust
let mut reader = stream.tag_reader();
let mut segmenter = m7s_plugin_hls::Segmenter::new(...);
loop { while let Some(tag) = reader.try_next() { ... } }
```

No `broadcast`/`subscribe` in HLS anymore; it consumes the lock-free ring via
`TagStream`.

Tests:

```text
hls::tests::segments_h264_keyframes ... ok
m7s-plugin-hls tests ... ok
```
## GB28181 SIP helper crate

`crates/m7s-plugin-gb28181` owns pure SIP helpers:

```text
tag_value / digest_param / md5_hex / uri_user
```

Engine `gb28181.rs` now imports these; the registrar/listener/PS media path
remains in the engine for the next split.

Tests: md5 vector, XML tag, URI user, digest parameter, plus all 35 root tests.
## GB28181 plugin crate complete

`src/gb28181.rs` is now a re-export of `m7s-plugin-gb28181`; the whole SIP
registrar, Catalog/INVITE, PS demux and RTP path moved into the crate.

Engine state now uses SDK traits:

```rust
GbState { streams: Arc<dyn StreamApi> }
process_payload(stream: &Arc<dyn TagSink>)
```

Tests: 12 (register 200/401/digest, Catalog, INVITE SDP, PS/RTP/ADTS,
expiry, unregister).
## RTSP helper crate

`crates/m7s-plugin-rtsp`:

```text
sdp(sps, pps, asc) -> String
rtcp_sr(ssrc, rtp_ts) -> [u8; 28]
```

Engine `rtsp.rs` delegates SDP and RTCP sender-report generation; RTSP
server/play loops remain in the engine until fully moved to `TagStream`.

Tests: SDP tracks, RTCP SR header, `rtsp_describe_setup_play`.
## RTSP TCP play loop on TagReader

`play_loop` now receives `Box<dyn m7s_sdk::TagReader>` instead of
`cache + broadcast`; the PLAY handler creates `stream.tag_reader()` for the
TCP path. UDP path still uses `subscribe`.

Verified: `rtsp_describe_setup_play` passes; RTP packetization unchanged.
## RTSP UDP play on TagReader

`play_udp` now also takes `Box<dyn TagReader>`; PLAY no longer calls
`stream.subscribe()` for RTSP. Both TCP and UDP egress paths consume the
lock-free ring through `TagStream`.

Verified: `rtsp_describe_setup_play`; root clippy/tests.
## TagStream snapshot + stronger StreamApi

`TagStream` adds:

```rust
fn snapshot(&self) -> Vec<Tag>;
```

`StreamApi::stream` now returns `Arc<dyn TagStream>` (instead of TagSink),
so protocol plugins can both push and read/snapshot. Engine `Stream`
implements `snapshot()` from the GOP cache.

GB28181/WebTransport/SRT adapt to the stronger trait; all tests pass.
This is the prerequisite for moving the RTSP server out of the engine.
## RTSP server crate complete

`src/rtsp.rs` is now a re-export of `m7s-plugin-rtsp`. The crate owns:

- TCP listener/session (OPTIONS/DESCRIBE/SETUP/PLAY/TEARDOWN);
- TCP interleaved RTP and UDP RTP;
- RTCP SR/RR and keepalive;
- SDP/RTCP/URL helpers.

It depends only on `m7s-sdk` (`StreamApi`, `TagStream`, `TagReader`) and
`m7s-codec`; DESCRIBE uses `TagStream::snapshot`, PLAY uses `tag_reader`.

Tests: 6 plugin tests + `rtsp_describe_setup_play`.
## Ring write benchmark

`examples/bench_ring.rs`, 1,000,000 writes, release:

```text
allocating Arc per write: 566.9 ns/write
preallocated Arc pool:    117.2 ns/write
```

The pooled number is the relevant one for the V6-style `Arc<AVFrame>` model
and is in the same ~100 ns/write class as Monibuca V6's published figure.
## TagStream tap (compatibility path)

`TagStream::tap()` returns a `broadcast::Receiver<SharedTag>` fed by the
engine only when a receiver exists. This preserves the exact behavior that
WHEP/WHIP previously got from `Stream::subscribe`, while `tag_reader`/ring
remain the primary path.

Test: `stream::tests::tag_stream_tap`.
## WHEP/WHIP plugin crate complete

`src/webrtc.rs` / `src/whip.rs` are re-exports of `m7s-plugin-webrtc`
(`whep`, `whip`). The crate depends only on `m7s-sdk` (`StreamApi`,
`TagStream`, `TagSink`) and `m7s-codec`.

WHEP uses `snapshot()` for config and `tap()` for live tags, which preserved
the previously verified browser decode path while removing `StreamManager`.

Verified with Chrome (single PeerConnection A/V):

```text
TRACK video / TRACK audio
ICE connected / PC connected
RTP video packets=1164
VIDEO 320x240 frames=206
AUDIO ready=4 time=7.95
```

WHIP canvas push:

```text
HTTP 201
PUSHED ice=connected
live/whip bytes=113270 cached_tags=116
```
## Cluster prerequisite: StreamInfo/listing

`m7s-sdk` now exposes:

```rust
pub struct StreamInfo { app, name, key, subscribers, bytes }
pub trait StreamApi {
    fn stream(...) -> Option<Arc<dyn TagStream>>;
    fn list(&self) -> Vec<StreamInfo>;
    fn count(&self) -> usize;
}
```

Engine `StreamManager` implements `list/count` from its session table.
Test: `stream::tests::stream_api_lists_sessions`.

This removes the last engine type dependency needed to move the cluster
control plane into `m7s-plugin-cluster`.
## Cluster plugin crate complete

`src/cluster.rs` is now a thin engine wrapper over `m7s-plugin-cluster`
(maps `ClusterConfig` to `m7s_sdk::ClusterOptions`).

The crate owns HTTP-FLV relay, QUIC relay, origin discovery, relay tracking,
sessions/servers/status/relays JSON.

SDK additions required:

```rust
fn get(app, name) -> Option<Arc<dyn TagStream>>; // non-creating
fn list() -> Vec<StreamInfo>;
fn count() -> usize;
```

Bug fixed during migration: `ensure_relay` must use non-creating `get`, not
`stream`, otherwise it returns early and never relays.

Verified: `cluster_flv::edge_pulls_flv_from_origin` passes.
## Web SDK + player page

- `web/sdk.js`: `M7S.Player` with WHEP negotiation, HLS/FLV native playback.
- `web/player.html`: minimal player UI, served at `/player`.
- `/sdk.js` serves the SDK with `application/javascript`.
- `/admin` remains read-only; player links can use `/player`.

Test: `tests/web_sdk.rs::serves_web_sdk` (200 + `M7S` + `whep`).
## Admin write operation: delete stream

- `StreamManager::remove(app,name)`.
- `GET /api/stream/delete?stream=app/name` -> `{"code":0,"removed":true|false}`.
- Admin table adds a Delete button per row.

Test (`tests/web_sdk.rs`): create `live/gone`, delete it, `/api/streams` returns
empty list.
## JWT login/auth

- `src/auth.rs`: HS256 JWT issue/verify, `login_json`.
- `GET /api/login?password=...` returns a token when
  `M7S_ADMIN_PASSWORD` matches.
- API accepts `Authorization: Bearer <jwt>` when `M7S_JWT_SECRET` is set,
  in addition to the static API token.
- `/api/login` is exempt from the auth gate.

Tests: `auth::tests::roundtrip`.
## Admin plugin configuration (persistent)

- `data/plugins.json` stores disabled plugin names
  (override path with `M7S_PLUGIN_CONFIG`).
- `GET /api/plugins/toggle?name=srt&enabled=0|1` updates it.
- `/api/plugins` reports file-disabled plugins.
- Startup gates (`m7s_rs::plugins::enabled`) honor the file.
- Changes take effect on next process start (listeners are bound at boot).

Tests: `plugins::tests::config_roundtrip`, `registry_has_26`.
## File-backed user store

- `data/users.json` (`M7S_USER_DB` override), SHA-256 hashes.
- `GET /api/login?username=&password=` checks the store; falls back to
  `M7S_ADMIN_PASSWORD` only when the store is empty.
- `GET /api/users/add?username=&password=` adds/replaces a user (API auth).
- JWT issued on successful login (`M7S_JWT_SECRET`).

Tests: `users::tests::add_and_check`, `auth::tests::roundtrip`.
## gRPC surface (minimal, real HTTP/2)

`src/grpc.rs`, TCP `0.0.0.0:50051`:

- `m7s.StreamService/ListStreams` -> `StreamList { Stream{app,name,subscribers,bytes} }`
- `grpc.health.v1.Health/Check`
- gRPC frame (5-byte header) + prost messages
- `grpc-status` trailers

Test `tests/grpc.rs::grpc_list_streams` uses a real h2 client and asserts
`live/grpc` is returned.
## gRPC GetStatus

Added `m7s.StreamService/GetStatus` -> `Status { streams, subscribers }`.
Test now sends two gRPC requests on one h2 connection and asserts both
`ListStreams` and `GetStatus`.
## MySQL user store

`src/mysql_users.rs` via `mysql_async`; enabled by `M7S_MYSQL_URL`.
Tested against MariaDB 11.4.5 (`mysql_user_roundtrip`), including login
fallback ordering. See `docs/MYSQL.md`.
## Admin UI details

- Token input persisted in `localStorage`; all fetches send
  `Authorization: Bearer <token>`.
- Plugin table with Enable/Disable calling `/api/plugins/toggle`.
- Stream Delete also sends the token.
- Link to `/player`.

Test `tests/web_sdk.rs` asserts `/admin` contains `Plugins` and
`Authorization`.
## fMP4 writer (building block)

`m7s-plugin-hls::fmp4`:

- `init_segment(sps,pps,w,h,timescale)` -> `ftyp+moov` (avc1/avcC).
- `media_segment(seq,base,samples,timescale)` -> `moof+mdat`.
- Unit tests validate box layout (`ftyp/moov/avcC`, `moof/trun/mdat`).

Status: writer only. It is **not** wired into the HLS playlist/endpoints yet,
so HLS fMP4/LL-HLS is still NOT complete.
## fMP4 HLS endpoint (video-only, on-demand)

- `GET /fmp4/<app>/<name>/index.m3u8` -> EXT-X-MAP playlist
- `GET .../init.mp4` -> ftyp+moov (mvex/trex)
- `GET .../seg.m4s` -> moof+mdat from the current GOP cache

Verified with a real RTMP H.264 push: concat init+seg, then

```text
ffmpeg -i fmp4_out.mp4 -f null -  -> exit 0
```

Only a duplicate-DTS warning at the concat boundary. Video-only; no audio,
no rolling media playlist, no LL-HLS parts yet.
## fMP4 multi-GOP playlist

`pack_all` splits cached video into GOP segments. Playlist now lists
`seg-0.m4s`, `seg-1.m4s`, ...; endpoint serves each by index.
Tests: `fmp4::tests::packs_synthetic_gop`, `splits_gops`.
## LL-HLS parts on fMP4 endpoint

Playlist adds `EXT-X-PART-INF`, `EXT-X-SERVER-CONTROL`, and one
`EXT-X-PART` per 0.5s chunk; parts are served as `part-<seg>-<part>.m4s`.

Tests: `fmp4::tests::splits_gops` asserts `EXT-X-PART`.
Not yet implemented: blocking playlist reload (`_HLS_msn`), part hold-back
behavior, independent part sequencing.
## Blocking playlist reload

`/fmp4/.../index.m3u8?_HLS_msn=N` waits up to 2s for `segs.len() > N`
(polling every 100ms) before responding.

Test `tests/fmp4_http.rs` covers `_HLS_msn=0` plus PART tags. The timed
wait path is implemented but not timing-asserted.
## fMP4 A/V endpoint

- `av_init.mp4`: ftyp+moov with video trak + audio `mp4a/esds` trak, two `trex`.
- `av.m4s`: one moof with two trafs + mdat.
- `av.m3u8`: EXT-X-MAP playlist.

Verified with real RTMP A/V: ffmpeg reports
`Stream #0:0 Video: h264 ... 320x240` and
`Stream #0:1 Audio: aac 44100 Hz stereo`.
Some H264 NAL errors remain when the cache snapshot starts mid-GOP; A/V box
layout and stream detection are confirmed.
## A/V multi-GOP fMP4

`pack_av_all` splits cached video by keyframe and assigns audio samples to each
GOP by timestamp. Playlist lists `av-<n>.m4s`; `av_init.mp4` contains both
tracks.

Tests: `fmp4::tests::av_all_has_segments`, `init_av_has_audio`.

Verified with FFmpeg: both video and audio streams detected; H264 NAL errors
still occur on some cache-boundary samples, so this is not yet clean A/V.
## A/V latest-GOP only

`pack_av_all` keeps only the newest keyframe GOP to avoid cache-boundary
samples. Still shows H264 NAL errors for A/V while video-only decodes clean;
layout/offset investigation continues.
## A/V diagnostic

A no-audio-traf experiment was reverted after exposing a slice-bounds panic
in the diagnostic path. Stable A/V output restored. H264 NAL errors remain
unresolved; video-only remains clean.
## A/V fMP4 fixed: full-box trex

Root cause: the second `trex` (audio track) in `init_segment_av` was written
with `boxed` instead of `full_box`, omitting version/flags. FFmpeg then
mis-parsed the movie extends and reported H264 NAL errors.

Fix: audio `trex` now uses `full_box(b"trex", 0, 0, ...)` (32 bytes each).
Test asserts both trex boxes are size 32.

Verified with real RTMP H.264+AAC: concat init+segment, `ffmpeg -f null -`
exits **0 with no H264 errors**.
## LL-HLS preload hint

Playlist now ends with:

```text
#EXT-X-PRELOAD-HINT:TYPE=PART,URI="part-<next>-0.m4s"
```

`EXT-X-ENDLIST` removed (live semantics). Test asserts the hint.
Still missing: `_HLS_part` blocking wait and actual preload part generation.
## LL-HLS `_HLS_part` wait

Playlist request accepts `_HLS_msn` + `_HLS_part`; waits up to 2s until the
requested segment/part exists (`segs.len() > msn`, or last segment has
`parts.len() > part`).

Test: `tests/fmp4_http.rs` requests `_HLS_msn=0&_HLS_part=0` and expects 200.
## Preload part endpoint

`/fmp4/.../part-<seg>-<part>.m4s` now waits up to 2s (100ms poll) for the
requested part to appear, matching the EXT-X-PRELOAD-HINT flow.

Test: `fmp4_http` requests `part-0-0.m4s` and asserts 200 + `moof`.
## Optimization: cached fMP4 pack

Before: every `/fmp4/...` playlist/init/part/segment request re-parsed the
GOP cache and re-muxed fMP4 (`pack_all`).

After: `pack_cached(key, tags)` memoizes init+segments by a cheap stamp
(tag count, last timestamp, last data length). Repeated requests reuse the
same `Arc<Packed>`; a new tag invalidates.

Test: `fmp4::tests::cache_reuses_and_invalidates`.
## Dispatcher implementation (2026-11-17)

`src/dispatcher.rs`:

- per-stream `Dispatcher`;
- `ArcSwap<Vec<Subscriber>>` lock-free list;
- per-subscriber bounded channel (150);
- `try_send` full -> dropped counter, never blocks publisher;
- `subscribe(backlog)` preloads GOP cache then live channel;
- `DispatchReader` unsubscribes on drop.

`Stream::push` now broadcasts through Dispatcher; `TagStream::tag_reader` and
HTTP-FLV use `dispatch_reader()` with cache backlog.

Tests:

```text
dispatcher::tests::broadcasts_to_all
dispatcher::tests::backlog_precedes_live
dispatcher::tests::drops_when_full_and_unsubscribes
rtsp_play / hls_http / cluster_flv / web_sdk integration
```

10K HTTP-FLV after Dispatcher:

```text
viewers=10000 ok=10000 failed=0 elapsed=15.54s
(previous 16.98s)
```
## DispatcherPool (2026-11-17)

`DispatcherPool::new(workers)`:

- 每个 worker 一条有界队列（4096）；
- 按 stream key 的 hash 选择 worker；
- worker 调用对应 Dispatcher.push；
- worker 队列满时回退到 inline dispatch，避免丢帧。

启用：`M7S_DISPATCHER_WORKERS=4`（0=关闭）。
测试：

```text
dispatcher::pool_tests::routes_and_delivers
dispatcher::pool_tests::same_key_same_worker
```

10K HTTP-FLV（4 workers）：**10000/10000, 15.54s**，与无池持平，
但把广播工作从发布者任务转移到 worker。
## WASM sandbox plugin runtime (2026-11-17)

`src/wasm_plugin.rs` using `wasmi` (no WASI/host imports):

- ABI: `memory`, `alloc(i32)->i32`, `transform(i32,i32)->i64` (`ptr<<32|len`);
- fuel limit 50M per call;
- input ≤64KiB, output ≤1MiB;
- `WasmRuntime` implements `PluginRuntime` and works with `PluginHost`.

Tests:

```text
wasm_plugin::tests::identity_transform
wasm_plugin::tests::rejects_garbage
wasm_plugin::runtime_tests::runtime_lifecycle_and_transform（含 PluginHost start/stop）
```
## WASM plugin load/process API (2026-11-17)

- `GET /api/wasm/load?name=&path=` (path relative to `M7S_WASM_DIR`, default `data/wasm`; canonical path must stay under root)
- `GET /api/wasm/list`
- `GET /api/wasm/process?name=&data=` -> `{"hex":...}`

Registry rejects invalid modules on load; process instantiates from stored bytes.
Integration: `tests/wasm_http.rs::wasm_load_process_list` (load .wat, process abc -> 616263, list contains name).
## WASM frame ABI + hot unload (2026-11-17)

- Optional export: `transform_frame(tag_type, timestamp, ptr, len) -> i64`;
- `WasmRuntime::process_frame`/`process_frame_registered` fall back to `transform` if absent;
- `GET /api/wasm/frame?name=&tag=&ts=&data=` -> hex;
- `GET /api/wasm/unload?name=` removes the registered module.

Tests: frame identity in `wasm_plugin::tests`, unload/frame in `tests/wasm_http.rs`.
## WASM dynamic enable/disable (2026-11-17)

- `GET /api/wasm/enable?name=&enabled=0|1`;
- disabled modules stay registered but `process`/`frame` return error;
- `/api/wasm/list` now returns `[{"name","enabled"}]`;
- unload removes both bytes and disabled flag.

Tests: `wasm_plugin::registry_tests::loads_and_processes` (disable/enable),
`tests/wasm_http.rs` (HTTP disable -> process error -> enable).
## WASM AV frame struct ABI (2026-11-17)

Memory layout (little-endian):

```text
0      tag_type u8
1      keyframe u8
2..4   reserved
4..8   timestamp u32
8..12  data_len u32
12..   data
```

Optional export `transform_av(ptr,len)->i64` (`ptr<<32|len`).
`process_av_frame` / `process_av_registered`; HTTP:

```text
GET /api/wasm/av?name=&tag=&ts=&key=&data=
```

Tests: AV roundtrip in `wasm_plugin::tests`, AV HTTP in `tests/wasm_http.rs`.
## WASM stream chain (2026-11-17)

- `run_chain(plugin, source, sink, stop)` reads source Dispatcher, converts
  tags to `AvFrame`, transforms, pushes to sink;
- `attach_registered(name, source, sink)` loads once and spawns;
- `GET /api/wasm/attach?name=&source=app/name&sink=app/name`.

Tests: `wasm_plugin::chain_tests::chains_frame_to_sink`,
`tests/wasm_http.rs` attach returns code 0.
## WASM chain stats/backpressure (2026-11-17)

- `ChainStats { processed, errors, stopped }`;
- 10 consecutive transform errors -> chain stops (`stopped=true`);
- `GET /api/wasm/chains` returns name/source/sink/processed/errors/stopped;
- attach registers stats keyed by `name:source->sink`.

Tests: `wasm_plugin::chain_tests` asserts processed>=1;
`tests/wasm_http.rs` asserts chains contains live/src.
## Downstream backpressure signal (2026-11-17)

`run_chain` compares `DispatchReader::dropped()` each loop:

- drop delta > 0 -> `backpressure++`, sleep 20ms, then continue;
- exposed in `/api/wasm/chains` as `"backpressure"`.

Unit coverage: `dispatcher::tests::drops_when_full_and_unsubscribes` (reader drop counter);
integration: `tests/wasm_http.rs` asserts `backpressure` field.
## WASM hot reload (2026-11-17)

- registry stores `(bytes, path)`;
- `GET /api/wasm/reload?name=` re-reads/validates the original file and swaps bytes;
- failed reload keeps the old module;
- `unload` removes bytes and disabled flag.

Tests: `wasm_plugin::registry_tests::loads_and_processes` (invalid reload rejected, valid reload succeeds),
`tests/wasm_http.rs` (reload endpoint).
## LL-HLS `_HLS_msn` blocking playlist (2026-11-18)

- `/fmp4/{app}/{name}/index.m3u8?_HLS_msn=N` waits until segment count > N;
- poll interval 100ms, timeout 5s then returns current playlist;
- query stripped before route matching, so `part-*`/`seg-*` unaffected.

Test: `tests/fmp4_http.rs::fmp4_playlist_blocking_reload` requests `_HLS_msn=0`,
checks `EXT-X-PART`/`PRELOAD-HINT` and part bytes.
## Foundation crates 5/5 (2026-11-18)

- `m7s-sdk`
- `m7s-codec`
- `m7s-config`
- `m7s-pool`（BytesPool，复用/上限）
- `m7s-task`（TaskGroup，spawn/cancel_all）

新增测试：

```text
m7s_pool::tests::reuses_and_caps
m7s_task::tests::cancels_all
```
## GB28181 MP4 playback + runtime controls (2026-11-18)

- `PlaybackControl`: pause/resume, seek, speed, stop（原子量）；
- `play_flv_bytes_live` / `play_flv_file_live` 支持运行中 seek/speed/pause；
- `play_mp4_file_live`: 通过 FFmpeg（`M7S_FFMPEG`，默认 `ffmpeg`）
  `-c copy -f flv pipe:1` 将 MP4 录制流转为 FLV 字节后播放，无需落盘；
- INVITE Playback 按文件扩展名选择 MP4/FLV 路径，并应用 START_MS/SPEED。

测试：

```text
ps::live_tests::pause_then_resume
ps::mp4_tests::mp4_remux_and_play
```
## gRPC GetStream (2026-11-18)

- `m7s.StreamService/GetStream` with `GetStreamRequest { key }`
- returns `Stream { app,name,subscribers,bytes }`; unknown key returns empty Stream.

Test: `tests/grpc.rs::grpc_list_streams` now also calls GetStream("live/grpc").
## gRPC ListPlugins (2026-11-18)

- `m7s.PluginService/ListPlugins` returns 26 `{name,module,enabled}` entries
  from the plugin registry.

Test: `tests/grpc.rs` decodes `PluginList`, asserts 26 and `rtmp` enabled.
## Metrics expansion (2026-11-18)

`/metrics` now includes:

```text
m7s_stream_bytes{key="app/name"} N
m7s_stream_subscribers{key="app/name"} N
m7s_dispatch_dropped_total N
```

`dispatcher::dropped_total()` aggregates bounded-channel drops.
Test: `tests/metrics_http.rs::metrics_expose_streams_and_dropped`.
## Live room persistence (2026-11-18)

- `Live::to_json/from_json/save/load`；
- `M7S_ROOMS_FILE` 设置后，`rooms()` 启动加载、join/leave 自动保存；
- 文件格式 `{"rooms":{"r":["alice"]},"capacity":1000}`。

测试：`persist_tests::save_and_load`。
## Official editions + gRPC RoomService (2026-11-18)

Editions:
- `edition_limit`: free/community 50, pro 1000, enterprise unlimited;
- `cluster_allowed_for`: only enterprise;
- `/api/license` includes `connection_limit` and `cluster`.

gRPC:
- `m7s.RoomService/ListRooms`
- `m7s.RoomService/JoinRoom`
- `m7s.RoomService/LeaveRoom`

Tests: `license::edition_tests`, `tests/grpc.rs` joins `grpc-room` and lists it.
## Room lifecycle APIs (2026-11-18)

- `GET /api/live/room/create?room=&title=`
- `GET /api/live/room/close?room=`
- `GET /api/live/rooms` -> name/title/created/closed/members

`RoomInfo` + create/close/list in `m7s-plugin-live`.
Tests: `lifecycle_tests::create_close_room`, `tests/room_http.rs::room_create_list_close`.
## SQLite metadata DB (2026-11-18)

- `m7s-rs` 依赖 `rusqlite`（bundled）；
- `MetaDb::open(path)` 建表 `m7s_streams`/`m7s_rooms`；
- `upsert_stream/list_streams/upsert_room/list_rooms`；
- `M7S_META_DB=<path>` 启用全局 DB；
- HTTP：
  ```text
  GET /api/db/streams
  GET /api/db/rooms
  ```

测试：`meta_db::tests::sqlite_roundtrip`、`tests/db_http.rs::db_streams_and_rooms`。
## Admin overview API (2026-11-18)

`GET /api/admin/overview` returns:

```json
{"node_id","streams","subscribers","plugins":{"total":26,"enabled":N},"rooms","db","license"}
```

Test: `tests/db_http.rs` checks total=26 and streams=1.
## React Admin (2026-11-18)

- `web/admin/`：React 18 + esbuild 工程；
- `src/App.jsx` 展示 overview/rooms/plugins；
- `npm run build` 生成 `web/admin.js`；
- HTTP：
  ```text
  GET /admin-react
  GET /admin.js
  ```

测试：

```text
cd web/admin && npm run build && node test.cjs
-> react_admin_ok（SSR 渲染包含 Monibuca Admin / 房间）
GET /admin-react /admin.js（Rust 集成测试）
```
## SRT TS demux test (2026-11-18)

`crates/m7s-plugin-srt` 增加 `TsMuxer` 驱动的单测：

- 生成 PAT/PMT + H.264 SPS/PPS/IDR 的 TS；
- `TsDemux::push/flush`；
- 断言产生视频 tag（type 9）。

```text
demux_tests::demuxes_ts_video_tag ... ok
```
## ONVIF service coverage (2026-11-18)

`tests::media2_and_events` 扩展覆盖：

- GetDeviceInformation
- GetCapabilities
- GetProfiles
- GetStreamUri
- GetVideoEncoderConfigurations / GetVideoSources / GetEventProperties
## WebTransport datagram E2E (2026-11-18)

`tests/webtransport.rs`：

- 启动 WebTransport server（UDP 4443）；
- 读取 `tests/fixtures/server.pem` 指纹信任；
- `ClientBuilder` 连接 `https://127.0.0.1:4443/live/wt`；
- 发送 datagram `hello-wt`；
- 断言 echo 相同。

```text
datagram_echo ... ok
```
## WebTransport media ingest (2026-11-18)

`tests/webtransport.rs` 扩展：

- 使用 `TsMuxer` 生成 PAT/PMT + H.264 SPS/PPS/IDR；
- 分片 datagram 发送；
- 等待后检查 stream 缓存包含 video tag（type 9）。

```text
datagram_echo ... ok
```
## RTSP UDP transport E2E (2026-11-18)

`tests/rtsp_play.rs::rtsp_udp_play`：

- SETUP `Transport: RTP/AVP;unicast;client_port=U-U+1`；
- PLAY；
- 客户端 UDP 收到 RTP，断言版本位 v2。

```text
rtsp_udp_play ... ok
```
## HLS AES-128 encryption (2026-11-18)

- `HlsConfig { encrypt, key }`（key 为 32 hex）；
- `HlsStream` 用 AES-128-CBC/PKCS7 加密每个 TS segment，IV=seq；
- playlist 注入 `#EXT-X-KEY:METHOD=AES-128,URI="key.bin"`；
- `GET /hls/{app}/{name}/key.bin` 返回 16B key。

测试：

```text
hls::encrypt_tests::encrypt_decrypt_roundtrip
hls::encrypt_tests::playlist_has_key
tests/hls_http.rs::hls_aes128_playlist_and_key
```
## HTTP API token enforcement test (2026-11-18)

`tests/auth_http.rs`：

- `/health` 无需 token -> 200；
- `/api/streams` 无 token -> 401；
- `Authorization: Bearer secret` -> 200；
- `X-API-Token: secret` -> 200；
- 错误 token -> 401。

```text
api_token_enforced ... ok
```
## Plugin registry acceptance (2026-11-18)

`tests/plugin_matrix.rs`：

- `/api/plugins` 返回 26 个插件；
- 名称唯一、module 非空；
- 包含 rtmp/hls/cluster-quic；
- toggle rtmp 关闭 -> status=disabled，再开启。

```text
plugin_matrix_26_and_toggle ... ok
```
## Runtime HLS plugin toggle (2026-11-18)

- `plugins::enabled` 现在同时检查 `M7S_PLUGINS_DISABLED` 与插件配置文件；
- `/hls/...` 请求实时检查 `enabled("hls")`，toggle 立即生效；
- 测试：`plugins::enabled_file_tests::toggle_affects_enabled`、
  `tests/plugin_matrix.rs`（toggle hls -> 404 -> enable -> 200）。
## Runtime listener toggle (2026-11-18)

- `m7s-sdk::plugins::enabled` 共享实现（env + 配置文件）；
- RTSP accept 处实时检查 `rtsp`，关闭后新连接直接丢弃；
- RTMP/RTMPS accept 处实时检查 `rtmp`；
- HLS HTTP 入口实时检查 `hls`。

测试：

```text
m7s_sdk::plugins::tests::env_disables
tests/rtsp_toggle.rs::rtsp_runtime_plugin_toggle
tests/plugin_matrix.rs（HLS toggle 200 -> 404 -> 200）
```
## WebTransport runtime toggle (2026-11-18)

- WebTransport session 建立前检查 `enabled("webtransport")`；
- 关闭时 respond `403 Forbidden`；
- 测试：`tests/webtransport.rs` 依次 [] -> 连接成功，["webtransport"] -> 连接失败，[] -> 恢复。
## QUIC/SRT runtime toggle (2026-11-18)

- QUIC cluster accept：`enabled("cluster-quic")` 为假时 `Incoming.refuse()`；
- SRT 主循环：`enabled("srt")` 为假时暂停处理 datagram；
- 测试：`tests/quic_relay.rs` 依次 [] 成功、["cluster-quic"] 拒绝、[] 恢复。
## RTSP UDP audio E2E (2026-11-18)

`tests/rtsp_play.rs::rtsp_udp_audio`：

- 推 AAC sequence header + AAC 帧；
- SETUP `trackID=1` + UDP client_port；
- PLAY；
- 断言收到 PT=97 的 RTP。

```text
rtsp_udp_audio ... ok
```
## Audio-only HLS (2026-11-18)

修复 `Segmenter::audio`：

- 音频配置到达时若未启用，自动开新 segment 并写 PAT/PMT；
- 音频帧按 target 时长切段（原来只有视频关键帧能切段，音频流永远无 segment）。

测试：`tests/hls_http.rs::hls_audio_only_ts`（AAC-only -> playlist + seg-0.ts）。
## Audio-only fMP4 HLS (2026-11-18)

- `init_segment_audio(asc, timescale)`；
- `media_segment_audio(sequence, base, samples)`；
- 引擎 `pack_audio_all`；
- 路由：
  ```text
  /fmp4/{app}/{name}/aud.m3u8
  /fmp4/{app}/{name}/aud_init.mp4
  /fmp4/{app}/{name}/aud-0.m4s
  ```

测试：

```text
m7s_plugin_hls::fmp4::audio_only_tests::audio_boxes
tests/fmp4_http.rs::fmp4_audio_only
```
## ONVIF PTZ coverage (2026-11-18)

`tests::media2_and_events` 增加：

- `ContinuousMove x=0.5 y=-0.5` -> ContinuousMoveResponse；
- `Stop` -> StopResponse。

并清理了重复的 Device/Capabilities/Profiles/StreamUri 断言。
## SRT publish E2E (2026-11-18)

`tests/srt_publish.rs`：

- `sapflow_srt::Caller` 握手到 SRT listener；
- `TsMuxer` 生成 10 帧 H.264 TS；
- 按 1024B、buffered<8 逐步 enqueue；
- poll_transmit/handle_datagram 驱动；
- 断言 stream 出现 video tag。

```text
srt_publish_ts ... ok
```
## SRT AES-128 publish E2E (2026-11-18)

`tests/srt_encrypted.rs`：

- 服务端 `M7S_SRT_PASSPHRASE=0123456789abcdef`；
- Caller `CryptoConfig(Aes128)`；
- TS 分片逐步 enqueue；
- 断言 stream 出现 video tag。

```text
SRT connect ... stream_id=live/srte
GOT_TAG
srt_publish_encrypted ... ok
```
## WebTransport stream ingest (2026-11-18)

`tests/webtransport.rs` 增加：

- 客户端 `session.open_bi()`；
- 写入完整 TS（SPS/PPS/IDR）；
- 服务端 accept_bi 读取并 TsDemux；
- 断言 stream 出现 video tag。

```text
datagram_echo ... ok（含 datagram echo + bi-stream TS ingest）
```
## RTSP TCP interleaved 音频（2026-11-18）

修复 `play_loop`：初始标签发送后 `flush`，实时标签发送后也 `flush`。
此前音频包（每包很小）可能留在 BufWriter 中，客户端收不到。

测试：`tests/rtsp_play.rs::rtsp_tcp_audio`

- SETUP trackID=1；
- PLAY 响应与 RTP 可能合并到同一 TCP 读，测试累积 buffer 查找 `$`；
- 断言 channel=2、PT=97。

```text
rtsp_tcp_audio ... ok
```
## gRPC CloseRoom + 加入房间可关闭（2026-11-18）

- `close_room` 现在对“仅 join 过、无 metadata”的房间也会创建 closed RoomInfo；
- 新增 `/m7s.RoomService/CloseRoom`；
- 测试：JoinRoom -> ListRooms -> CloseRoom(ok=true) -> 再次关闭 false。

```text
tests/grpc.rs ... ok
```
## gRPC CreateRoom（2026-11-18）

- 新增 `/m7s.RoomService/CreateRoom`（room=名称，user=title）；
- 测试顺序：CreateRoom -> JoinRoom -> ListRooms -> CloseRoom。

```text
tests/grpc.rs ... ok
```
## gRPC DeleteStream（2026-11-18）

- `StreamApi` 增加默认 `remove()`；
- 根 `StreamManager` 实现 remove；
- gRPC `/m7s.StreamService/DeleteStream`（key="app/name"）返回 `RoomReply.ok`；
- 测试：ListStreams -> DeleteStream(live/grpc) -> ok=true。

```text
tests/grpc.rs ... ok
```
## gRPC CreateStream（2026-11-18）

- 新增 `/m7s.StreamService/CreateStream`（key="app/name"）；
- 调用 `StreamApi::stream` 创建/获取，返回 ok；
- 测试在 DeleteStream 之后创建 live/new。

```text
tests/grpc.rs ... ok
```