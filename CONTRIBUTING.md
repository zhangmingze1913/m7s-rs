# Contributing

## Build

```powershell
cargo build --release --workspace
```

## Required checks

```powershell
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
node web/sdk_test.cjs
node web/admin/test.cjs
```

Performance-sensitive changes should also run:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\perf_gate.ps1 -Viewers 5000
```

## Scope and style

- Keep protocol/plugin code inside its crate; root crate is the engine and HTTP
  control plane.
- Hot paths (HTTP-FLV, dispatcher, tag serialization) must avoid per-frame
  allocations and per-connection polling.
- Add a regression test for behavior changes; performance changes need before/
  after numbers in the PR.
- Update `docs/ACCEPTANCE_MATRIX.md` and `docs/V6_DIFF_2026-11-18.md` when a
  capability or known limitation changes.
- Do not commit production certificates, private keys, `data/`, `target/`, or
  bundled Chrome/VLC binaries.

## Test fixtures

`tests/fixtures/*-key.pem` and `ca.pem` are ephemeral test material. They are
not production credentials.
