# Test TLS fixtures

These files are **test-only** material used by TLS/mTLS/RTMPS/gRPC tests:

- `ca.pem`
- `server.pem`, `server-key.pem`
- `client.pem`, `client-key.pem`

They are not production credentials and must not be deployed. Do not replace
them with real keys. If a scanner flags `*-key.pem`, treat it as an intentional
test fixture; `SECURITY.md` documents this.
