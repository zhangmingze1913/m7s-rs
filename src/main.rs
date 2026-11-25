use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::TcpListener;

use m7s_rs::cluster::Cluster;
use m7s_rs::config::Config;
use m7s_rs::flv::FlvTag;
use m7s_rs::gb28181;
use m7s_rs::hls::HlsManager;
use m7s_rs::http::{AppState, run as http_run, run_tls};
use m7s_rs::onvif;
use m7s_rs::quic_cluster;
use m7s_rs::record::RecordManager;
use m7s_rs::rtmp;
use m7s_rs::rtsp;
use m7s_rs::srt;
use m7s_rs::stream::StreamManager;
use m7s_rs::tls;
use m7s_rs::webrtc;
use m7s_rs::webtransport;
use m7s_rs::grpc;
use m7s_rs::whip;

#[derive(Debug, Parser)]
#[command(
    name = "m7s-rs",
    version,
    about = "Rust RTMP/HTTP-FLV server with Origin/Edge cluster"
)]
struct Cli {
    #[arg(short, long, default_value = "config/server.toml")]
    config: PathBuf,
    #[arg(long)]
    duration: Option<u64>,
    /// Push a synthetic H.264/AAC stream to live/test for load tests.
    #[arg(long)]
    demo: bool,
}

struct CtxEvents;

impl m7s_sdk::EventSink for CtxEvents {
    fn emit(&self, topic: &str, payload: &str) {
        m7s_rs::events::publish(topic, payload);
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(4)
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(cli.config, cli.duration, cli.demo)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("server error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(path: PathBuf, duration: Option<u64>, demo: bool) -> Result<()> {
    let cfg = Config::load(&path)?;
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .try_init()
        .ok();
    let workers = std::env::var("M7S_DISPATCHER_WORKERS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    m7s_rs::dispatcher::init_pool(workers);
    if let Ok(v) = std::env::var("M7S_WASM_HOT_RELOAD") {
        m7s_rs::wasm_plugin::start_watcher(v.parse().unwrap_or(5));
    }
    let streams = StreamManager::new(cfg.server.gop_cache);
    let cluster = Cluster::new(cfg.cluster.clone(), streams.clone());
    let hls = HlsManager::new(streams.clone(), cfg.hls.clone());
    let recorder = RecordManager::new(streams.clone(), cfg.record.clone());
    let state = AppState {
        streams: streams.clone(),
        cluster,
        hls,
        node_id: cfg.cluster.node_id.clone(),
        api_token: cfg.auth.api_token.clone(),
    };
    let http_listener = bind_listener(&cfg.server.http_bind, cfg.server.backlog)?;
    let rtmp_listener = bind_listener(&cfg.server.rtmp_bind, cfg.server.backlog)?;
    let rtsp_listener = if cfg.rtsp.enabled {
        Some(bind_listener(&cfg.rtsp.bind, cfg.server.backlog)?)
    } else {
        None
    };
    let http_addr = http_listener.local_addr()?;
    let rtmp_addr = rtmp_listener.local_addr()?;
    let scheme = if cfg.tls.enabled { "https" } else { "http" };
    println!("HTTP/FLV : {scheme}://{http_addr}/flv/live/test.flv");
    println!("RTMP      : rtmp://{rtmp_addr}/live/test");
    println!("API       : {scheme}://{http_addr}/api/streams");
    if cfg.rtsp.enabled {
        println!("RTSP      : rtsp://{}/live/test", cfg.rtsp.bind);
    }
    println!(
        "cluster   : node={} advertise={}",
        cfg.cluster.node_id, cfg.cluster.advertise
    );

    let cfg_rtmp = cfg.clone();
    let tls_acceptor = if cfg.tls.enabled {
        Some(if cfg.tls.require_client_cert {
            tls::load_server_with_client_auth(
                std::path::Path::new(&cfg.tls.cert),
                std::path::Path::new(&cfg.tls.key),
                std::path::Path::new(&cfg.tls.client_ca),
            )?
        } else {
            tls::load_server(
                std::path::Path::new(&cfg.tls.cert),
                std::path::Path::new(&cfg.tls.key),
            )?
        })
    } else {
        None
    };
    if demo {
        let demo_streams = streams.clone();
        tokio::spawn(demo_source(demo_streams));
    }
    let rtmp_tls = if cfg.tls.rtmp {
        tls_acceptor.clone()
    } else {
        None
    };
    tokio::spawn(async move {
        let result = if let Some(acceptor) = rtmp_tls {
            rtmp::run_tls(rtmp_listener, streams, cfg_rtmp, recorder, acceptor).await
        } else {
            rtmp::run(rtmp_listener, streams, cfg_rtmp, recorder).await
        };
        if let Err(e) = result {
            log::error!("RTMP server stopped: {e:#}");
        }
    });
    let mut ctx = m7s_sdk::EngineContext::new(&cfg.cluster.node_id, &format!("http://{}", cfg.server.http_bind), &format!("rtsp://{}", cfg.rtsp.bind)).with_streams(state.streams.clone()).with_events(std::sync::Arc::new(CtxEvents)).with_http(std::sync::Arc::new(m7s_rs::http_client::HttpClientImpl));
    if let Some(db) = m7s_rs::meta_db::global() {
        ctx = ctx.with_meta(db.clone());
    }
    if cfg.gb28181.enabled {
        let socket = tokio::net::UdpSocket::bind(&cfg.gb28181.bind).await?;
        println!("GB28181 SIP: {}", socket.local_addr()?);
        let state = gb28181::GbState::with_context(
            &cfg.gb28181.domain,
            &cfg.gb28181.password,
            cfg.gb28181.media_port,
            ctx.clone(),
        );
        tokio::spawn(async move {
            if let Err(e) = gb28181::run(socket, state).await {
                log::error!("GB28181 stopped: {e:#}");
            }
        });
    }
    if let Some(listener) = rtsp_listener {
        let rtsp_ctx = ctx.clone();
        tokio::spawn(async move {
            if let Err(e) = rtsp::run_with_context(listener, rtsp_ctx).await {
                log::error!("RTSP server stopped: {e:#}");
            }
        });
    }
    let whep_listener = bind_listener("0.0.0.0:8081", cfg.server.backlog)?;

    if m7s_rs::plugins::enabled("webrtc-whep") && m7s_rs::license::premium_allowed() {
        let whep_ctx = ctx.clone();
        tokio::spawn(async move {
            if let Err(e) = webrtc::run_with_context(whep_listener, whep_ctx).await {
                log::error!("WHEP server stopped: {e:#}");
            }
        });
    } else {
        drop(whep_listener);
    }
    let whip_listener = bind_listener("0.0.0.0:8082", cfg.server.backlog)?;

    if m7s_rs::plugins::enabled("webrtc-whip") && m7s_rs::license::premium_allowed() {
        let whip_ctx = ctx.clone();
        tokio::spawn(async move {
            if let Err(e) = whip::run_with_context(whip_listener, whip_ctx).await {
                log::error!("WHIP server stopped: {e:#}");
            }
        });
    } else {
        drop(whip_listener);
    }
    let onvif_listener = bind_listener("0.0.0.0:8083", cfg.server.backlog)?;
    if m7s_rs::plugins::enabled("onvif") && m7s_rs::license::premium_allowed() {
        tokio::spawn(async move {
            if let Err(e) = onvif::run(onvif_listener).await {
                log::error!("ONVIF server stopped: {e:#}");
            }
        });
        tokio::spawn(async move {
            if let Err(e) = onvif::discovery().await {
                log::error!("ONVIF discovery stopped: {e:#}");
            }
        });
    } else {
        drop(onvif_listener);
    }
    let quic_streams = state.streams.clone();
    let quic_advertise = cfg.cluster.advertise.clone();
    if m7s_rs::plugins::enabled("cluster-quic") && m7s_rs::license::cluster_allowed() {
        tokio::spawn(async move {
            if let Err(e) = quic_cluster::run(quic_streams, quic_advertise).await {
                log::error!("QUIC cluster stopped: {e:#}");
            }
        });
    }
    if m7s_rs::plugins::enabled("webtransport") && m7s_rs::license::premium_allowed() {
        let wt_ctx = ctx.clone();
        tokio::spawn(async move {
            if let Err(e) = webtransport::run_with_context("0.0.0.0:4443", wt_ctx).await {
                log::error!("WebTransport stopped: {e:#}");
            }
        });
    }
    if m7s_rs::plugins::enabled("srt") && m7s_rs::license::premium_allowed() {
        let srt_ctx = ctx.clone();
        tokio::spawn(async move {
            if let Err(e) = srt::run_with_context("0.0.0.0:9000", srt_ctx).await {
                log::error!("SRT server stopped: {e:#}");
            }
        });
    }
    if m7s_rs::plugins::enabled("grpc") {
        let grpc_listener = bind_listener("0.0.0.0:50051", cfg.server.backlog)?;
        let grpc_ctx = ctx.clone();
        let grpc_tls = if cfg.tls.enabled && cfg.tls.grpc { tls_acceptor.clone() } else { None };
        tokio::spawn(async move {
            let result = if let Some(acceptor) = grpc_tls {
                grpc::run_tls_with_context(grpc_listener, grpc_ctx, acceptor).await
            } else {
                grpc::run_with_context(grpc_listener, grpc_ctx).await
            };
            if let Err(e) = result {
                log::error!("gRPC stopped: {e:#}");
            }
        });
    }
    let state_http = state.clone();
    tokio::spawn(async move {
        let result = match tls_acceptor {
            Some(acceptor) => run_tls(http_listener, state_http, acceptor).await,
            None => http_run(http_listener, state_http).await,
        };
        if let Err(e) = result {
            log::error!("HTTP server stopped: {e:#}");
        }
    });

    let started = tokio::time::Instant::now();
    let deadline = duration.map(|s| started + Duration::from_secs(s));
    loop {
        if let Some(deadline) = deadline {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    println!("m7s-rs stopped");
    let _ = state.streams.count();
    Ok(())
}

#[allow(dead_code)]
fn bind_listener(addr: &str, backlog: u32) -> Result<TcpListener> {
    let sa: std::net::SocketAddr = addr.parse()?;
    let socket = Socket::new(Domain::for_address(sa), Type::STREAM, Some(Protocol::TCP))?;
    socket.set_reuse_address(true)?;
    socket.bind(&sa.into())?;
    socket.listen(backlog.min(i32::MAX as u32) as i32)?;
    socket.set_nonblocking(true)?;
    Ok(TcpListener::from_std(socket.into())?)
}
#[allow(dead_code)]
fn stdin_wait() {
    let mut buf = [0u8; 1];
    let _ = std::io::stdin().read(&mut buf);
}

async fn demo_source(streams: Arc<StreamManager>) {
    let stream = streams.get_or_create("live", "test");
    let config = vec![
        0x01, 0x42, 0x00, 0x1e, 0xff, 0xe1, 0x00, 0x04, 0x67, 0x42, 0x00, 0x1e, 0x01, 0x00, 0x03,
        0x68, 0xce, 0x06, 0xe2,
    ];
    let mut seq = vec![0x17, 0, 0, 0, 0];
    seq.extend_from_slice(&config);
    stream.push(Arc::new(FlvTag::new(9, 0, seq)));
    stream.push(Arc::new(FlvTag::new(8, 0, vec![0xaf, 0, 0x12, 0x10])));
    let mut ts = 0u32;
    let mut frame = 0u32;
    loop {
        let key = frame % 25 == 0;
        let payload = vec![if key { 0x65 } else { 0x41 }, 1, 2, 3];
        let mut data = vec![if key { 0x17 } else { 0x27 }, 1, 0, 0, 0, 0, 0, 0, 0x04];
        data.extend_from_slice(&payload);
        stream.push(Arc::new(FlvTag::new(9, ts, data)));
        stream.push(Arc::new(FlvTag::new(8, ts, vec![0xaf, 1, 1, 2, 3])));
        ts = ts.wrapping_add(40);
        frame = frame.wrapping_add(1);
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

#[allow(dead_code)]
fn require<T>(value: Option<T>, message: &str) -> Result<T> {
    value.context(message.to_string())
}
