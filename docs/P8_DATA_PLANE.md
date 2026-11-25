# P8 Data Plane: Lock-Free RingBuffer

## Implemented

`src/ring.rs`:

- fixed-capacity, power-of-two SPMC ring;
- one writer, many independent readers;
- `Arc<T>` frame sharing (no frame bytes copy);
- per-slot `AtomicU64` sequence + `ArcSwapOption` publication;
- overwrite-oldest semantics;
- `RingReader::try_next` / `read`;
- `dropped` counter.

Integrated into `Stream`:

- every `push` also publishes to the ring;
- `Stream::ring_reader()` returns an independent zero-copy reader;
- test `ring_shares_arc` asserts `Arc::ptr_eq`.

## Tests

```text
ring::tests::write_read
ring::tests::overwrite_oldest
ring::tests::multiple_readers
ring::tests::concurrent_arc_sharing
stream::tests::ring_shares_arc
36 unit tests pass
```

## Next

- move HTTP-FLV/QUIC/SRT subscribers to `RingReader`;
- add `AVFrame`/Track model above the ring;
- benchmark write latency P50/P99.

## Subscriber migration

`serve_flv` now uses `Stream::ring_reader()` instead of
`RwLock<VecDeque>` cache + `broadcast` receiver:

- initial GOP/config replay from the ring;
- independent reader cursor per HTTP-FLV client;
- 20 ms idle poll;
- subscriber count is incremented on `ring_reader()` and released with
  `Stream::leave()`.

Integration tests (`http_flv`, `cluster_flv`) still pass, so the HTTP-FLV wire
format and cache behavior are unchanged.
## QUIC relay migration

`quic_cluster` origin now streams from `Stream::ring_reader()` instead of
`broadcast`. Verified with `examples/quic_edge.rs`:

```text
QUIC_EDGE tags=50 bytes=40927
```
## WHEP migration attempt (reverted)

WHEP video/audio were switched to `RingReader`; browser received RTP video
packets but decoded 0 frames (SPS/PPS/keyframe ordering regression at ring
start). The change was reverted to the previously verified
`cache + broadcast` path. WHEP stays on broadcast until ring GOP/config
replay is redesigned; HTTP-FLV and QUIC keep the ring path.
## WHEP migration second attempt (reverted)

Tried `cache_snapshot + subscribe_latest` with config replay; Chrome still
received RTP video but decoded 0 frames. Reverted to the verified
`subscribe + broadcast` path. Root cause is not simply missing SPS/PPS;
it needs a dedicated GOP-aware ring subscription with keyframe anchoring and
sequence continuity before WHEP can move.