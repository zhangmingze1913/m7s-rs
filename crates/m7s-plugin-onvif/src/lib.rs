//! Minimal ONVIF Device Service over HTTP SOAP.
//!
//! Supports GetDeviceInformation, GetCapabilities and GetStreamUri.

pub mod client;

use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ring::digest;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub async fn run_with_context(listener: TcpListener, ctx: m7s_sdk::EngineContext) -> Result<()> {
    let _ = ctx;
    run(listener).await
}

pub async fn run(listener: TcpListener) -> Result<()> {
    log::info!("ONVIF listening on {}", listener.local_addr()?);
    loop {
        let (socket, peer) = listener.accept().await?;
        let Some(guard) = m7s_sdk::connections::try_acquire() else {
            drop(socket);
            continue;
        };
        tokio::spawn(async move {
            let _guard = guard;
            if let Err(e) = handle(socket).await {
                log::debug!("ONVIF {peer}: {e:#}");
            }
        });
    }
}

async fn handle(mut socket: TcpStream) -> Result<()> {
    let mut head = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        head.extend_from_slice(&buf[..n]);
        if head.windows(4).any(|w| w == b"\r\n\r\n") || head.len() > 128 * 1024 {
            break;
        }
    }
    let pos = head
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("ONVIF headers")?;
    let headers = String::from_utf8_lossy(&head[..pos]).to_string();
    let mut body = head[pos + 4..].to_vec();
    let mut content_len = 0usize;
    for line in headers.lines() {
        if let Some(v) = line.strip_prefix("Content-Length:") {
            content_len = v.trim().parse().unwrap_or(0);
        }
    }
    while body.len() < content_len {
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
    }
    let soap = String::from_utf8_lossy(&body);
    if !authorized(&soap) {
        let out = "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/soap+xml\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        socket.write_all(out.as_bytes()).await?;
        socket.flush().await?;
        return Ok(());
    }
    let reply = if soap.contains("GetDeviceInformation") {
        envelope(
            "<tds:GetDeviceInformationResponse>\
             <tds:Manufacturer>m7s-rs</tds:Manufacturer>\
             <tds:Model>m7s-rs</tds:Model>\
             <tds:FirmwareVersion>0.1.0</tds:FirmwareVersion>\
             <tds:SerialNumber>m7s-rs-0001</tds:SerialNumber>\
             <tds:HardwareId>rust</tds:HardwareId>\
             </tds:GetDeviceInformationResponse>",
        )
    } else if soap.contains("GetCapabilities") {
        envelope(
            "<tds:GetCapabilitiesResponse><tds:Capabilities>\
             <tt:Device><tt:XAddr>http://127.0.0.1:8083/onvif/device_service</tt:XAddr></tt:Device>\
             <tt:Media><tt:XAddr>http://127.0.0.1:8083/onvif/media_service</tt:XAddr></tt:Media>\
             </tds:Capabilities></tds:GetCapabilitiesResponse>",
        )
    } else if soap.contains("GetProfiles") {
        envelope(
            r#"<trt:GetProfilesResponse><trt:Profiles token="profile1" fixed="true"><tt:Name>main</tt:Name><tt:VideoSourceConfiguration token="vsc1"><tt:Name>video</tt:Name><tt:SourceToken>vsrc1</tt:SourceToken></tt:VideoSourceConfiguration><tt:VideoEncoderConfiguration token="vec1"><tt:Name>h264</tt:Name><tt:Encoding>H264</tt:Encoding><tt:Resolution><tt:Width>320</tt:Width><tt:Height>240</tt:Height></tt:Resolution></tt:VideoEncoderConfiguration></trt:Profiles></trt:GetProfilesResponse>"#,
        )
    } else if soap.contains("GetVideoEncoderConfigurations") {
        envelope(
            r#"<trt:GetVideoEncoderConfigurationsResponse><trt:Configurations token="vec1"><tt:Name>h264</tt:Name><tt:UseCount>1</tt:UseCount><tt:Encoding>H264</tt:Encoding><tt:Resolution><tt:Width>320</tt:Width><tt:Height>240</tt:Height></tt:Resolution><tt:RateControl><tt:FrameRateLimit>25</tt:FrameRateLimit><tt:BitrateLimit>800</tt:BitrateLimit></tt:RateControl></trt:Configurations></trt:GetVideoEncoderConfigurationsResponse>"#,
        )
    } else if soap.contains("GetVideoSources") {
        envelope(
            r#"<trt:GetVideoSourcesResponse><trt:VideoSources token="vsrc1"><tt:Framerate>25</tt:Framerate><tt:Resolution><tt:Width>320</tt:Width><tt:Height>240</tt:Height></tt:Resolution></trt:VideoSources></trt:GetVideoSourcesResponse>"#,
        )
    } else if soap.contains("GetEventProperties") {
        envelope(
            r#"<tev:GetEventPropertiesResponse><tev:TopicSet><tt:TopicSet><tt:Topic>tns1:VideoSource/MotionAlarm</tt:Topic></tt:TopicSet></tev:TopicSet></tev:GetEventPropertiesResponse>"#,
        )
    } else if soap.contains("GetStreamUri") {
        let uri = std::env::var("M7S_ONVIF_STREAM_URI")
            .unwrap_or_else(|_| "rtsp://127.0.0.1:8554/live/test".to_string());
        envelope(&format!(
            "<trt:GetStreamUriResponse><trt:MediaUri>\
             <tt:Uri>{uri}</tt:Uri>\
             <tt:InvalidAfterConnect>false</tt:InvalidAfterConnect>\
             </trt:MediaUri></trt:GetStreamUriResponse>"
        ))
    } else if soap.contains("ContinuousMove") {
        let x = attr(&soap, "x").unwrap_or(0.0);
        let y = attr(&soap, "y").unwrap_or(0.0);
        *ptz().lock().unwrap_or_else(|e| e.into_inner()) = (x, y, 0.0);
        envelope("<tptz:ContinuousMoveResponse/>")
    } else if soap.contains("Stop") {
        *ptz().lock().unwrap_or_else(|e| e.into_inner()) = (0.0, 0.0, 0.0);
        envelope("<tptz:StopResponse/>")
    } else if soap.contains("GetStatus") {
        let (x, y, z) = *ptz().lock().unwrap_or_else(|e| e.into_inner());
        envelope(&format!(
            "<tptz:GetStatusResponse><tptz:PTZStatus><tt:Position>\
             <tt:PanTilt x=\"{x:.3}\" y=\"{y:.3}\"/><tt:Zoom x=\"{z:.3}\"/>\
             </tt:Position><tt:MoveStatus><tt:PanTilt>IDLE</tt:PanTilt>\
             <tt:Zoom>IDLE</tt:Zoom></tt:MoveStatus></tptz:PTZStatus>\
             </tptz:GetStatusResponse>"
        ))
    } else if soap.contains("GetServices") {
        envelope(
            "<tds:GetServicesResponse>\
             <tds:Service><tds:Namespace>http://www.onvif.org/ver10/device/wsdl</tds:Namespace><tds:XAddr>http://127.0.0.1:8083/onvif/device_service</tds:XAddr></tds:Service>\
             <tds:Service><tds:Namespace>http://www.onvif.org/ver10/media/wsdl</tds:Namespace><tds:XAddr>http://127.0.0.1:8083/onvif/media_service</tds:XAddr></tds:Service>\
             <tds:Service><tds:Namespace>http://www.onvif.org/ver20/ptz/wsdl</tds:Namespace><tds:XAddr>http://127.0.0.1:8083/onvif/ptz_service</tds:XAddr></tds:Service>\
             </tds:GetServicesResponse>",
        )
    } else if soap.contains("GetServiceCapabilities") {
        envelope("<tds:GetServiceCapabilitiesResponse><tds:Capabilities NetworkHostname=\"m7s-rs\"/></tds:GetServiceCapabilitiesResponse>")
    } else if soap.contains("GetScopes") {
        envelope("<tds:GetScopesResponse><tds:Scopes><tt:ScopeDef>Fixed</tt:ScopeDef><tt:ScopeItem>onvif://www.onvif.org/name/m7s-rs</tt:ScopeItem></tds:Scopes></tds:GetScopesResponse>")
    } else if soap.contains("GetSystemDateAndTime") {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        envelope(&format!(
            "<tds:GetSystemDateAndTimeResponse><tds:SystemDateAndTime><tt:DateTimeType>Manual</tt:DateTimeType><tt:UTCDateTime><tt:Time><tt:Hour>{}</tt:Hour><tt:Minute>{}</tt:Minute><tt:Second>{}</tt:Second></tt:Time></tt:UTCDateTime></tds:SystemDateAndTime></tds:GetSystemDateAndTimeResponse>",
            (now / 3600) % 24,
            (now / 60) % 60,
            now % 60
        ))
    } else if soap.contains("GetNetworkInterfaces") {
        envelope("<tds:GetNetworkInterfacesResponse><tds:NetworkInterfaces token=\"eth0\"><tt:Enabled>true</tt:Enabled><tt:Info><tt:Name>eth0</tt:Name></tt:Info></tds:NetworkInterfaces></tds:GetNetworkInterfacesResponse>")
    } else if soap.contains("GetVideoSourceConfigurations") {
        envelope(r#"<trt:GetVideoSourceConfigurationsResponse><trt:Configurations token="vsc1"><tt:Name>video</tt:Name><tt:SourceToken>vsrc1</tt:SourceToken></trt:Configurations></trt:GetVideoSourceConfigurationsResponse>"#)
    } else if soap.contains("GetAudioSources") {
        envelope(r#"<trt:GetAudioSourcesResponse><trt:AudioSources token="asrc1"><tt:Channels>1</tt:Channels><tt:Samplerate>8000</tt:Samplerate></trt:AudioSources></trt:GetAudioSourcesResponse>"#)
    } else if soap.contains("GetAudioEncoderConfigurations") {
        envelope(r#"<trt:GetAudioEncoderConfigurationsResponse><trt:Configurations token="aec1"><tt:Name>aac</tt:Name><tt:Encoding>AAC</tt:Encoding><tt:Bitrate>64</tt:Bitrate><tt:SampleRate>44100</tt:SampleRate></trt:Configurations></trt:GetAudioEncoderConfigurationsResponse>"#)
    } else if soap.contains("GetVideoEncoderConfigurationOptions") {
        envelope(r#"<trt:GetVideoEncoderConfigurationOptionsResponse><trt:Options><tt:H264><tt:ResolutionsAvailable><tt:Width>320</tt:Width><tt:Height>240</tt:Height></tt:ResolutionsAvailable></tt:H264></trt:Options></trt:GetVideoEncoderConfigurationOptionsResponse>"#)
    } else if soap.contains("SetVideoEncoderConfiguration") {
        envelope("<trt:SetVideoEncoderConfigurationResponse/>")
    } else if soap.contains("AbsoluteMove") {
        let x = tag_attr(&soap, "tt:PanTilt", "x").unwrap_or(0.0);
        let y = tag_attr(&soap, "tt:PanTilt", "y").unwrap_or(0.0);
        let z = tag_attr(&soap, "tt:Zoom", "x").unwrap_or(0.0);
        *ptz().lock().unwrap_or_else(|e| e.into_inner()) = (x, y, z);
        envelope("<tptz:AbsoluteMoveResponse/>")
    } else if soap.contains("RelativeMove") {
        let mut p = ptz().lock().unwrap_or_else(|e| e.into_inner());
        p.0 += tag_attr(&soap, "tt:PanTilt", "x").unwrap_or(0.0);
        p.1 += tag_attr(&soap, "tt:PanTilt", "y").unwrap_or(0.0);
        p.2 += tag_attr(&soap, "tt:Zoom", "x").unwrap_or(0.0);
        envelope("<tptz:RelativeMoveResponse/>")
    } else if soap.contains("GetConfigurations") {
        envelope("<tptz:GetConfigurationsResponse><tptz:PTZConfiguration token=\"ptz1\"><tt:Name>ptz</tt:Name><tt:DefaultAbsolutePantTiltPositionSpace>http://www.onvif.org/ver10/tptz/PanTiltSpaces/PositionGenericSpace</tt:DefaultAbsolutePantTiltPositionSpace></tptz:PTZConfiguration></tptz:GetConfigurationsResponse>")
    } else if soap.contains("GetPresets") {
        let presets = presets().lock().unwrap_or_else(|e| e.into_inner());
        let items: String = presets
            .iter()
            .map(|(token, (x, y, z))| format!(
                "<tptz:Preset token=\"{token}\"><tt:Name>{token}</tt:Name><tt:PTZPosition><tt:PanTilt x=\"{x:.3}\" y=\"{y:.3}\"/><tt:Zoom x=\"{z:.3}\"/></tt:PTZPosition></tptz:Preset>"
            ))
            .collect();
        envelope(&format!("<tptz:GetPresetsResponse>{items}</tptz:GetPresetsResponse>"))
    } else if soap.contains("SetPreset") {
        let token = tag(&soap, "<tptz:PresetToken>", "</tptz:PresetToken>")
            .or_else(|| tag(&soap, "<PresetToken>", "</PresetToken>"))
            .map(str::to_string)
            .unwrap_or_else(|| format!("preset{}", presets().lock().unwrap_or_else(|e| e.into_inner()).len() + 1));
        let pos = *ptz().lock().unwrap_or_else(|e| e.into_inner());
        presets().lock().unwrap_or_else(|e| e.into_inner()).insert(token.clone(), pos);
        envelope(&format!("<tptz:SetPresetResponse><tptz:PresetToken>{token}</tptz:PresetToken></tptz:SetPresetResponse>"))
    } else if soap.contains("GotoPreset") {
        let token = tag(&soap, "<tptz:PresetToken>", "</tptz:PresetToken>")
            .or_else(|| tag(&soap, "<PresetToken>", "</PresetToken>"))
            .unwrap_or("");
        if let Some(pos) = presets().lock().unwrap_or_else(|e| e.into_inner()).get(token).copied() {
            *ptz().lock().unwrap_or_else(|e| e.into_inner()) = pos;
        }
        envelope("<tptz:GotoPresetResponse/>")
    } else if soap.contains("RemovePreset") {
        if let Some(token) = tag(&soap, "<tptz:PresetToken>", "</tptz:PresetToken>")
            .or_else(|| tag(&soap, "<PresetToken>", "</PresetToken>"))
        {
            presets().lock().unwrap_or_else(|e| e.into_inner()).remove(token);
        }
        envelope("<tptz:RemovePresetResponse/>")
    } else if soap.contains("CreatePullPointSubscription") {
        envelope("<tev:CreatePullPointSubscriptionResponse><tev:SubscriptionReference><wsa:Address>http://127.0.0.1:8083/onvif/events</wsa:Address></tev:SubscriptionReference><tev:CurrentTime>2026-11-21T00:00:00Z</tev:CurrentTime><tev:TerminationTime>2026-11-21T01:00:00Z</tev:TerminationTime></tev:CreatePullPointSubscriptionResponse>")
    } else if soap.contains("PullMessages") {
        let events: Vec<Event> = {
            let mut q = events().lock().unwrap_or_else(|e| e.into_inner());
            q.drain(..).collect()
        };
        let msgs: String = events
            .iter()
            .map(|e| {
                format!(
                    "<tev:NotificationMessage><tev:Topic>{}</tev:Topic><tev:Message><tt:Message><tt:Source><tt:SimpleItem Name=\"State\" Value=\"{}\"/></tt:Source></tt:Message></tev:Message></tev:NotificationMessage>",
                    xml_escape(&e.topic),
                    xml_escape(&e.value)
                )
            })
            .collect();
        envelope(&format!("<tev:PullMessagesResponse><tev:CurrentTime>2026-11-21T00:00:00Z</tev:CurrentTime><tev:TerminationTime>2026-11-21T01:00:00Z</tev:TerminationTime>{msgs}</tev:PullMessagesResponse>"))
    } else {
        envelope("<tds:Fault><tds:Reason>ActionNotSupported</tds:Reason></tds:Fault>")
    };
    let out = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/soap+xml; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        reply.len(),
        reply
    );
    socket.write_all(out.as_bytes()).await?;
    socket.flush().await?;
    Ok(())
}

fn ptz() -> &'static Mutex<(f32, f32, f32)> {
    static PTZ: OnceLock<Mutex<(f32, f32, f32)>> = OnceLock::new();
    PTZ.get_or_init(|| Mutex::new((0.0, 0.0, 0.0)))
}

fn tag_attr(text: &str, tag_name: &str, name: &str) -> Option<f32> {
    let start = text.find(&format!("<{tag_name}"))?;
    let end = start + text[start..].find('>')?;
    let inside = &text[start..=end];
    let pattern = format!("{name}=\"");
    let vstart = inside.find(&pattern)? + pattern.len();
    let vend = inside[vstart..].find('"')? + vstart;
    inside[vstart..vend].parse().ok()
}

type PresetMap = std::collections::HashMap<String, (f32, f32, f32)>;

fn presets() -> &'static Mutex<PresetMap> {
    static PRESETS: OnceLock<Mutex<PresetMap>> = OnceLock::new();
    PRESETS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn attr(soap: &str, name: &str) -> Option<f32> {
    let pattern = format!("{name}=\"");
    let start = soap.find(&pattern)? + pattern.len();
    let end = soap[start..].find('"')? + start;
    soap[start..end].parse().ok()
}

fn tag<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(&text[start..end])
}

fn authorized(soap: &str) -> bool {
    let Ok(user) = std::env::var("M7S_ONVIF_USER") else {
        return true;
    };
    let pass = std::env::var("M7S_ONVIF_PASSWORD").unwrap_or_default();
    let Some(got_user) = tag(soap, "<wsse:Username>", "</wsse:Username>") else {
        return false;
    };
    if got_user != user {
        return false;
    }
    let (Some(nonce), Some(created), Some(digest_b64)) = (
        tag(soap, "<wsse:Nonce", "</wsse:Nonce>"),
        tag(soap, "<wsu:Created>", "</wsu:Created>"),
        tag(soap, "<wsse:Password", "</wsse:Password>"),
    ) else {
        return false;
    };
    let nonce = nonce.split('>').next_back().unwrap_or("");
    let digest_b64 = digest_b64.split('>').next_back().unwrap_or("");
    let Ok(nonce_raw) = STANDARD.decode(nonce.trim()) else {
        return false;
    };
    let mut input = nonce_raw;
    input.extend_from_slice(created.as_bytes());
    input.extend_from_slice(pass.as_bytes());
    let hash = digest::digest(&digest::SHA1_FOR_LEGACY_USE_ONLY, &input);
    STANDARD.encode(hash.as_ref()) == digest_b64.trim()
}

fn envelope(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"
 xmlns:tds="http://www.onvif.org/ver10/device/wsdl"
 xmlns:tt="http://www.onvif.org/ver10/schema"
 xmlns:trt="http://www.onvif.org/ver10/media/wsdl"
 xmlns:tptz="http://www.onvif.org/ver20/ptz/wsdl"
 xmlns:tev="http://www.onvif.org/ver10/events/wsdl"
 xmlns:tr2="http://www.onvif.org/ver20/media/wsdl">
<s:Body>{body}</s:Body></s:Envelope>"#
    )
}

pub async fn discovery() -> Result<()> {
    let s = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    s.set_reuse_address(true)?;
    s.bind(&"0.0.0.0:3702".parse::<std::net::SocketAddr>()?.into())?;
    s.set_nonblocking(true)?;
    let socket = tokio::net::UdpSocket::from_std(s.into())?;
    socket
        .join_multicast_v4("239.255.255.250".parse()?, "0.0.0.0".parse()?)
        .ok();
    log::info!("ONVIF WS-Discovery listening on 0.0.0.0:3702");
    let xaddr = format!("http://{}:8083/onvif/device_service", local_ip());
    let mut buf = [0u8; 8192];
    loop {
        let Ok((n, peer)) = socket.recv_from(&mut buf).await else {
            continue;
        };
        let text = String::from_utf8_lossy(&buf[..n]);
        if !text.contains("Probe") || text.contains("ProbeMatches") {
            continue;
        }
        let reply = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"
 xmlns:a="http://schemas.xmlsoap.org/ws/2004/08/addressing"
 xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery"
 xmlns:dn="http://www.onvif.org/ver10/network/wsdl">
<s:Header><a:Action>http://schemas.xmlsoap.org/ws/2005/04/discovery/ProbeMatches</a:Action>
<a:MessageID>uuid:m7s-rs-probe-match</a:MessageID>
<a:RelatesTo>uuid:m7s-rs</a:RelatesTo></s:Header>
<s:Body><d:ProbeMatches><d:ProbeMatch>
<d:Types>dn:NetworkVideoTransmitter</d:Types>
<d:Scopes>onvif://www.onvif.org/type/video_encoder onvif://www.onvif.org/name/m7s-rs</d:Scopes>
<d:XAddrs>{xaddr}</d:XAddrs>
<d:MetadataVersion>1</d:MetadataVersion>
</d:ProbeMatch></d:ProbeMatches></s:Body></s:Envelope>"#
        );
        let _ = socket.send_to(reply.as_bytes(), peer).await;
    }
}

fn local_ip() -> String {
    if let Ok(host) = std::env::var("M7S_ONVIF_HOST") {
        return host;
    }
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn post(addr: std::net::SocketAddr, body: String) -> String {
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        let req = format!(
            "POST /onvif/device_service HTTP/1.1\r\nHost: x\r\nContent-Type: application/soap+xml\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        String::from_utf8_lossy(&out).to_string()
    }

    #[test]
    fn media2_and_events() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                tokio::spawn(run(listener));
                let a = post(addr, "<GetVideoEncoderConfigurations/>".to_string()).await;
                assert!(a.contains("GetVideoEncoderConfigurationsResponse"), "{a}");
                let b = post(addr, "<GetVideoSources/>".to_string()).await;
                assert!(b.contains("GetVideoSourcesResponse"), "{b}");
                let c = post(addr, "<GetEventProperties/>".to_string()).await;
                assert!(c.contains("GetEventPropertiesResponse"), "{c}");
                let d = post(addr, "<GetDeviceInformation/>".to_string()).await;
                assert!(d.contains("GetDeviceInformationResponse"), "{d}");
                let e = post(addr, "<GetCapabilities/>".to_string()).await;
                assert!(e.contains("GetCapabilitiesResponse"), "{e}");
                let f = post(addr, "<GetProfiles/>".to_string()).await;
                assert!(f.contains("GetProfilesResponse"), "{f}");
                let g = post(addr, "<GetStreamUri/>".to_string()).await;
                assert!(g.contains("GetStreamUriResponse"), "{g}");
                let h = post(addr, "<ContinuousMove><x>0.5</x><y>-0.5</y></ContinuousMove>".to_string()).await;
                assert!(h.contains("ContinuousMoveResponse"), "{h}");
                let i = post(addr, "<Stop/>".to_string()).await;
                assert!(i.contains("StopResponse"), "{i}");
            });
    }
    #[test]
    fn onvif_extended_surface() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                tokio::spawn(run(listener));
                for action in [
                    "GetServices",
                    "GetServiceCapabilities",
                    "GetScopes",
                    "GetSystemDateAndTime",
                    "GetNetworkInterfaces",
                    "GetVideoSourceConfigurations",
                    "GetAudioSources",
                    "GetAudioEncoderConfigurations",
                    "GetVideoEncoderConfigurationOptions",
                    "SetVideoEncoderConfiguration",
                    "GetConfigurations",
                    "CreatePullPointSubscription",
                    "PullMessages",
                ] {
                    let reply = post(addr, format!("<{action}/>")).await;
                    assert!(reply.contains(&format!("{action}Response")), "{action}: {reply}");
                }
                let move_reply = post(
                    addr,
                    "<AbsoluteMove><tt:Position><tt:PanTilt x=\"0.5\" y=\"-0.25\"/><tt:Zoom x=\"0.1\"/></tt:Position></AbsoluteMove>".to_string(),
                )
                .await;
                assert!(move_reply.contains("AbsoluteMoveResponse"), "{move_reply}");
                let status = post(addr, "<GetStatus/>".to_string()).await;
                assert!(status.contains("x=\"0.500\""), "{status}");
                assert!(status.contains("y=\"-0.250\""), "{status}");
                let preset = post(addr, "<SetPreset><PresetToken>p1</PresetToken></SetPreset>".to_string()).await;
                assert!(preset.contains("SetPresetResponse"), "{preset}");
                let presets = post(addr, "<GetPresets/>".to_string()).await;
                assert!(presets.contains("token=\"p1\""), "{presets}");
                let goto = post(addr, "<GotoPreset><PresetToken>p1</PresetToken></GotoPreset>".to_string()).await;
                assert!(goto.contains("GotoPresetResponse"), "{goto}");
            });
    }
}

#[derive(Clone)]
struct Event {
    topic: String,
    value: String,
}

fn events() -> &'static Mutex<std::collections::VecDeque<Event>> {
    static EVENTS: OnceLock<Mutex<std::collections::VecDeque<Event>>> = OnceLock::new();
    EVENTS.get_or_init(|| Mutex::new(std::collections::VecDeque::new()))
}

pub fn push_event(topic: &str, value: &str) {
    let mut q = events().lock().unwrap_or_else(|e| e.into_inner());
    q.push_back(Event { topic: topic.to_string(), value: value.to_string() });
    while q.len() > 100 {
        q.pop_front();
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
