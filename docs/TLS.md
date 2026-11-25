# TLS / RTMPS / mTLS

## 1. 内置 HTTPS

```toml
[tls]
enabled = true
cert = "config/cert.pem"
key = "config/key.pem"
```

已验证：`tests/tls_http.rs` 真实 TLS 握手 + `/health` 200。

## 2. RTMPS

```toml
[tls]
enabled = true
rtmp = true
cert = "config/cert.pem"
key = "config/key.pem"
```

已验证：

- `tests/rtmps.rs`：TLS 握手 + RTMP connect/createStream/publish；
- FFmpeg 实际推流：`ffmpeg ... -f flv rtmps://127.0.0.1:1936/live/tls` 成功进入 StreamManager；
- FFmpeg HTTPS-FLV 拉流：`-tls_verify 0 -i https://.../flv/live/tls.flv` 可解码。

## 3. mTLS

```toml
[tls]
enabled = true
require_client_cert = true
client_ca = "config/client-ca.pem"
```

实现：`rustls::server::WebPkiClientVerifier`，从 `client_ca` 加载 CA，要求客户端证书。

已通过 `tests/mtls_http.rs`：CA 签发客户端证书可访问，未携带证书被拒绝。生产仍需补：

- CA/客户端证书签发；
- 无证书拒绝、过期证书拒绝、错误 CA 拒绝；
- 证书吊销/轮换。

## 4. 生成测试证书

```bash
cargo run --example gen_cert
```

生成 `config/cert.pem` / `config/key.pem`（仅开发）。生产使用企业 CA/Let's Encrypt。

## 5. 未完成

- mTLS 客户端证书联调；
- 证书热加载；
- HSTS/CORS 白名单/限流；
- RTMPS 与 HTTP TLS 使用同一证书，暂不支持分离。
