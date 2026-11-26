# Dynamic Plugins (C ABI v1)

## Loading

Set `M7S_PLUGIN_PATH` to a `.dll` / `.so`:

```powershell
$env:M7S_PLUGIN_PATH = "plugins/hello/target/release/m7s_plugin_hello.dll"
```

Loader: `libloading` 0.8, `src/dynamic.rs`.

## ABI

```c
uint32_t m7s_plugin_abi(void);
const char *m7s_plugin_name(void);
intptr_t m7s_transform(const uint8_t *in, size_t len,
                       uint8_t *out, size_t cap);
```

`m7s_transform` returns bytes written or `-1`.

## Endpoints

```text
GET /api/dynamic/info
GET /api/dynamic?msg=hello
```

Verified:

```json
{"abi":1,"loaded":true,"name":"hello"}
{"echo":"HELLO","loaded":true}
```

## Sample

`plugins/hello` is a `cdylib` crate that uppercases ASCII.

Build:

```powershell
cargo build --manifest-path plugins/hello/Cargo.toml --release
```

## Limits

- ABI is unstable across compiler versions; plugins must be rebuilt.
- One plugin per process (extend to a directory scan later).

## WASM market / hot reload（2026-11-21）

- `wasm_plugin::market()` 扫描 `M7S_WASM_DIR` 的 `.wasm/.wat`：
  name/file/size/installed/enabled；
- `GET /api/wasm/market`；
- `wasm_plugin::hot_reload()` 对比已安装文件字节，校验后原子替换；
- `GET /api/wasm/hot_reload`；
- `M7S_WASM_HOT_RELOAD=<secs>` 启动后台轮询 watcher；
`M7S_WASM_MEMORY_MB` 限制每个 WASM 实例线性内存（默认 64MB），`M7S_WASM_FUEL` 限制指令数。

测试：

```text
wasm_plugin::registry_tests::loads_and_processes（含 market/hot_reload）
tests/wasm_http.rs::wasm_load_process_list（含 market/hot_reload HTTP）
```

签名/权限/资源配额仍未实现。

## WASM Ed25519 ǩ����2026-11-22��

```text
M7S_WASM_PUBKEY=<64 hex chars>
<plugin>.wat.sig
```

- ���ù�Կ��`load_file` �����ҵ�ͬ�� `.sig` ��ͨ�� Ed25519 У�飻
- ȱʧ/����ǩ���ܾ����أ�
- ���ԣ�`wasm_plugin::registry_tests::loads_and_processes`��ǩ��/�۸ģ���

## WASM 权限/配额（2026-11-22）

`	ext
M7S_WASM_ALLOW=plugin-a,plugin-b
M7S_WASM_FUEL=50000000
`

- 白名单为空=全部允许；
- 加载时校验名字；
- wasmi fuel 配额；
- GET /api/wasm/permissions 返回 allow/fuel/require_signature。
测试：	ests/wasm_http.rs、wasm_plugin::registry_tests::allowed_list。
