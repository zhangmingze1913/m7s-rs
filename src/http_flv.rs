//! HTTP-FLV hot path.
//!
//! This module is deliberately separate from the control-plane HTTP routes in
//! `src/http.rs`: it is the per-viewer data path, so it must avoid allocations
//! and per-connection polling. Comments below explain each performance-
//! sensitive step instead of restating Rust syntax.

use std::time::Duration;

use anyhow::Result;
use serde_json::json;
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::http::{AppState, respond_json, respond_redirect};

/// Serve one HTTP-FLV viewer.
///
/// Protocol shape:
/// - HTTP headers;
/// - FLV file header (9 bytes + PreviousTagSize);
/// - a continuous stream of FLV tags until the client disconnects.
pub async fn serve<S>(mut socket: S, state: AppState, app: &str, name: &str) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    // Edge node: pull the stream from the origin only when it is not local.
    // `ensure_relay` is idempotent; a failed relay leaves the stream absent so
    // the 404/redirect branch below can still run.
    if state.streams.get(app, name).is_none() {
        let _ = state.cluster.ensure_relay(app, name).await;
    }

    // Resolve the local stream after relay attempt. If still absent, prefer a
    // deterministic 307 shard redirect over a misleading 404.
    let Some(stream) = state.streams.get(app, name) else {
        if let Some(location) = state.cluster.redirect_flv(app, name) {
            return respond_redirect(&mut socket, &location).await;
        }
        return respond_json(
            &mut socket,
            404,
            &json!({"code":404,"message":"stream not found"}),
        )
        .await;
    };

    // Write fixed headers once. `Connection: close` keeps lifecycle simple:
    // a viewer is exactly one TCP connection and one dispatch subscriber.
    socket
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: video/x-flv\r\nCache-Control: no-cache\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        )
        .await?;
    socket
        .write_all(&crate::flv::flv_header(true, true))
        .await?;

    // One subscriber cursor per viewer. The dispatcher uses a shared broadcast
    // ring, so creating this reader does not create a per-viewer frame queue.
    let mut reader = stream.dispatch_reader();

    // 8 KiB is enough for a typical A/V frame pair. Starting at 64 KiB made
    // 20K connections reserve ~1.2 GiB of virtual buffers and raised RSS a lot.
    let mut out = Vec::with_capacity(8 * 1024);

    // Latency is recorded for every 64th tag per viewer. The histogram is
    // global per stream and lock-protected, so sampling prevents 10K viewers
    // from serializing on it while retaining thousands of p99 samples.
    let mut seen: u64 = 0;

    loop {
        // Block until the next tag arrives. This replaces the old 20ms poll:
        // no timer wakeups while a stream is idle, and lower delivery latency.
        let Some(tag) = reader.recv().await else {
            break;
        };

        if seen >= 16 && seen & 63 == 0 {
            stream.record_latency(tag.created.elapsed().as_micros().min(u32::MAX as u128) as u32);
        }
        seen = seen.wrapping_add(1);

        // Serialize the tag directly into the reusable buffer. `write_to` avoids
        // the temporary `Vec` that `to_bytes()` allocates per tag.
        tag.write_to(&mut out);

        // Drain everything already queued before touching the socket. Video and
        // audio for the same frame usually arrive together, so this halves the
        // number of syscalls without adding a batching delay.
        while let Some(tag) = reader.try_next() {
            if seen >= 16 && seen & 63 == 0 {
                stream.record_latency(
                    tag.created.elapsed().as_micros().min(u32::MAX as u128) as u32,
                );
            }
            seen = seen.wrapping_add(1);
            tag.write_to(&mut out);

            // Bound memory if a slow client accumulates many tags. On a write
            // failure the viewer is gone; release its subscriber immediately.
            if out.len() >= 64 * 1024 {
                if socket.write_all(&out).await.is_err() {
                    stream.leave();
                    return Ok(());
                }
                out.clear();
            }
        }

        // Flush the coalesced frame(s). A broken pipe means the viewer closed;
        // `reader` Drop unsubscribes from the dispatcher.
        if socket.write_all(&out).await.is_err() {
            break;
        }
        out.clear();
    }

    // Explicitly decrement the stream subscriber counter even when the reader
    // is dropped later by the function epilogue.
    stream.leave();
    Ok(())
}

/// Keep a named import so this module documents the original timeout constant
/// used by older polling code and by callers that re-export it.
#[allow(dead_code)]
const LEGACY_POLL_INTERVAL: Duration = Duration::from_millis(20);
