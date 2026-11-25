use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn config(http: u16, rtmp: u16, rtsp: u16, node: &str, seeds: &str) -> String {
    format!(
        r#"[server]
http_bind = "127.0.0.1:{http}"
rtmp_bind = "127.0.0.1:{rtmp}"
gop_cache = 500
backlog = 1024

[tls]
enabled = false
cert = "config/cert.pem"
key = "config/key.pem"

[rtsp]
enabled = true
bind = "127.0.0.1:{rtsp}"

[gb28181]
enabled = false
bind = "127.0.0.1:0"
domain = "3402000000"
password = ""

[hls]
enabled = false
segment_secs = 4
max_segments = 6

[record]
enabled = false
dir = "{}/m7s-failover-record"
[cluster]
enabled = true
node_id = "{node}"
advertise = "127.0.0.1:{http}"
seeds = [{seeds}]
heartbeat_secs = 5
relay_timeout_secs = 5

[auth]
publish_key = ""
api_token = ""
"#,
        std::env::temp_dir().display().to_string().replace('\\', "/")
    )
}

fn spawn(bin: &str, cfg: &str, demo: bool) -> Child {
    let mut cmd = Command::new(bin);
    cmd.args(["--config", cfg, "--duration", "90"]);
    if demo {
        cmd.arg("--demo");
    }
    cmd.env(
        "M7S_PLUGINS_DISABLED",
        "webrtc-whep,webrtc-whip,onvif,webtransport,srt,cluster-quic,grpc",
    )
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .unwrap()
}

fn wait_health(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = s.write_all(b"GET /health HTTP/1.1\r\nHost: t\r\n\r\n");
            let mut b = [0u8; 64];
            if let Ok(n) = s.read(&mut b) {
                if String::from_utf8_lossy(&b[..n]).contains("200") {
                    return;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("health timeout on {port}");
}

fn read_bytes(stream: &mut TcpStream, want: usize, timeout: Duration) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    while out.len() < want && Instant::now() < deadline {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => break,
        }
    }
    out
}

#[test]
#[ignore = "spawns two server processes; run serially with --ignored"]
fn origin_process_restart_edge_recovers() {
    let bin = env!("CARGO_BIN_EXE_m7s-rs");
    let o_http = free_port();
    let o_rtmp = free_port();
    let o_rtsp = free_port();
    let e_http = free_port();
    let e_rtmp = free_port();
    let e_rtsp = free_port();
    let dir = std::env::temp_dir();
    let o_cfg = dir.join(format!("m7s-o-{}.toml", std::process::id()));
    let e_cfg = dir.join(format!("m7s-e-{}.toml", std::process::id()));
    std::fs::write(
        &o_cfg,
        config(o_http, o_rtmp, o_rtsp, "origin", ""),
    )
    .unwrap();
    std::fs::write(
        &e_cfg,
        config(
            e_http,
            e_rtmp,
            e_rtsp,
            "edge",
            &format!("\"127.0.0.1:{o_http}\""),
        ),
    )
    .unwrap();

    let mut origin = spawn(bin, o_cfg.to_str().unwrap(), true);
    wait_health(o_http);
    let mut edge = spawn(bin, e_cfg.to_str().unwrap(), false);
    wait_health(e_http);

    let mut client = TcpStream::connect(("127.0.0.1", e_http)).unwrap();
    client
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    client
        .write_all(b"GET /flv/live/test.flv HTTP/1.1\r\nHost: e\r\n\r\n")
        .unwrap();
    let first = read_bytes(&mut client, 256, Duration::from_secs(20));
    assert!(first.len() > 13, "edge did not relay: {}", first.len());

    origin.kill().unwrap();
    let _ = origin.wait();
    std::thread::sleep(Duration::from_millis(800));

    let mut origin2 = spawn(bin, o_cfg.to_str().unwrap(), true);
    wait_health(o_http);

    let more = read_bytes(&mut client, first.len() + 256, Duration::from_secs(30));
    assert!(
        more.len() > first.len(),
        "edge did not recover after origin restart: {} -> {}",
        first.len(),
        more.len()
    );

    let _ = origin2.kill();
    let _ = origin2.wait();
    let _ = edge.kill();
    let _ = edge.wait();
    let _ = std::fs::remove_file(&o_cfg);
    let _ = std::fs::remove_file(&e_cfg);
}
