# P6 Built-in Plugins (26)

The build is a compile-time plugin composition. `GET /api/plugins` returns
this registry with `count: 26`.

| # | Plugin | Module | Capability |
|---|---|---|---|
| 1 | rtmp | `rtmp` | RTMP ingest, chunk/AMF0 |
| 2 | http-flv | `http` | HTTP-FLV output |
| 3 | hls | `hls` | HLS playlist/segments |
| 4 | rtsp | `rtsp` | RTSP TCP/UDP, RTP/RTCP |
| 5 | gb28181 | `gb28181` | SIP, Catalog, PS/RTP |
| 6 | webrtc-whep | `webrtc` | WHEP H.264/PCMU |
| 7 | webrtc-whip | `whip` | WHIP H.264 ingest |
| 8 | srt | `srt` | SRT, TS demux, AES |
| 9 | webtransport | `webtransport` | QUIC/HTTP3 media |
| 10 | onvif-device | `onvif` | ONVIF Device Service |
| 11 | onvif-media | `onvif` | Profiles/StreamUri |
| 12 | onvif-discovery | `onvif` | WS-Discovery |
| 13 | cluster-http | `cluster` | HTTP cluster control |
| 14 | cluster-quic | `quic_cluster` | QUIC relay |
| 15 | record | `record` | FLV recording |
| 16 | admin | `http` | Admin page |
| 17 | auth | `http` | API token/publish key |
| 18 | tls | `tls` | HTTPS/RTMPS |
| 19 | metrics | `http` | Prometheus metrics |
| 20 | streams-api | `http` | Stream/session API |
| 21 | mpegts-mux | `ts` | MPEG-TS mux |
| 22 | mpegts-demux | `srt` | PAT/PMT/PES demux |
| 23 | rtp-rtcp | `rtsp` | RTP/RTCP packetization |
| 24 | flv-mux | `flv` | FLV tags |
| 25 | avc-config | `rtsp` | SPS/PPS/AVC config |
| 26 | hls-segmenter | `hls` | Segment cache |

## Notes

- This is an internal module registry, not dynamic `.so` loading.
- `GET /api/plugins` is the machine-readable counterpart.
- New plugins require recompilation.

## Enable / disable

Set `M7S_PLUGINS_DISABLED` to a comma-separated list before startup:

```powershell
$env:M7S_PLUGINS_DISABLED = "srt,onvif,webtransport,cluster-quic,webrtc-whep,webrtc-whip"
```

`GET /api/plugins` reports `status: disabled` for those entries, and the
corresponding listeners are not started.
## Live gifts / PK state

`m7s-plugin-live` adds:

- `Gifts::send(room,user,gift,count)` -> total, keeps last gift log;
- `Gifts::total/last`;
- `Pk::add(room,side,points)` and `Pk::score(room)`.

Test: `gift_tests::gift_and_pk`.
## Live room HTTP API

- `GET /api/live/join?room=&user=`
- `GET /api/live/leave?room=&user=`
- `GET /api/live/gift?room=&user=&gift=&count=`
- `GET /api/live/pk?room=&side=&points=`
- `GET /api/live/status?room=`

Test `tests/web_sdk.rs`: join -> gift(total 2) -> status(members 1).
## Meeting room HTTP API

```text
GET /api/meeting/join?room=&user=
GET /api/meeting/leave?room=&user=
GET /api/meeting/publish?room=&user=&on=1|0
GET /api/meeting/status?room=
```

Test `tests/web_sdk.rs`: join -> members 1, publish -> publishers 1.
## Room list APIs

- `GET /api/live/rooms` -> non-empty live rooms
- `GET /api/meeting/rooms` -> non-empty meeting rooms

Test `tests/web_sdk.rs` asserts `"r"` and `"m"` after joins.
## Meeting mix/record core

`m7s-plugin-mix`:

- `mix(inputs, output, secs)` -> hstack/xstack MP4 via FFmpeg.
- `mix_bytes(inputs, secs)` -> fragmented MP4 over stdout (testable in sandbox).

Test `mix_bytes_tests::mixes_two_sources_to_bytes`: two `lavfi` sources
mixed, output starts with `ftyp`.
File output is sandbox-blocked in tests; production path unchanged.
## Meeting stream mapping / recording

- `GET /api/meeting/stream?room=&stream=app/name` sets mapping;
- `GET /api/meeting/stream?room=` returns mapping;
- `GET /api/meeting/record?room=&secs=` records the mapped stream via MP4 plugin.

Test: `tests/web_sdk.rs` sets `live/cam` and reads it back.