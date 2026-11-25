#!/usr/bin/env python3
"""Open N HTTP-FLV viewers with asyncio and report success.

This is a lab load generator, not a 10k-stream benchmark. Start the server with
`--demo` first:

    m7s-rs --config config/server.toml --demo --duration 30
    python scripts/load_test.py 1000 10
"""
import asyncio
import sys
import time
from urllib.parse import urlparse

target = sys.argv[1] if len(sys.argv) > 1 else "127.0.0.1:8180/live"
count = int(sys.argv[2]) if len(sys.argv) > 2 else 100
seconds = int(sys.argv[3]) if len(sys.argv) > 3 else 10
parsed = urlparse(target if "://" in target else "http://" + target)
host = parsed.hostname or "127.0.0.1"
port = parsed.port or 8180
path = "/flv" + (parsed.path or "/live").rstrip("/") + "/test.flv"
ok = 0
failed = 0
lock = asyncio.Lock()


async def viewer(index: int) -> None:
    global ok, failed
    writer = None
    try:
        reader, writer = await asyncio.wait_for(
            asyncio.open_connection(host, port), timeout=5
        )
        request = (
            f"GET {path} HTTP/1.1\r\nHost: {host}\r\n"
            "Connection: close\r\n\r\n"
        )
        writer.write(request.encode())
        await writer.drain()
        header = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), timeout=5)
        if b"200" not in header:
            raise RuntimeError("not 200")
        first = await asyncio.wait_for(reader.read(4096), timeout=5)
        if b"FLV" not in first:
            raise RuntimeError("no FLV")
        async with lock:
            ok += 1
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            chunk = await asyncio.wait_for(reader.read(65536), timeout=5)
            if not chunk:
                break
    except Exception:
        async with lock:
            failed += 1
    finally:
        if writer is not None:
            writer.close()
            try:
                await writer.wait_closed()
            except Exception:
                pass


async def main() -> None:
    start = time.monotonic()
    # Limit concurrent handshakes so the accept queue is not the bottleneck.
    semaphore = asyncio.Semaphore(200)

    async def limited(index: int) -> None:
        async with semaphore:
            await viewer(index)

    await asyncio.gather(*(limited(i) for i in range(count)))
    print(
        f"viewers={count} ok={ok} failed={failed} "
        f"elapsed={time.monotonic() - start:.2f}s "
        f"url=http://{host}:{port}{path}"
    )


asyncio.run(main())
