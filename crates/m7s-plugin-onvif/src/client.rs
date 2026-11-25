//! Minimal ONVIF client (Device/Media/PTZ SOAP over HTTP).

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Clone)]
pub struct OnvifClient {
    pub endpoint: String,
    pub user: Option<(String, String)>,
}

impl OnvifClient {
    pub fn new(endpoint: &str) -> Self {
        Self { endpoint: endpoint.to_string(), user: None }
    }

    pub fn with_auth(mut self, user: &str, password: &str) -> Self {
        self.user = Some((user.to_string(), password.to_string()));
        self
    }

    pub async fn call(&self, body: &str) -> Result<String> {
        let rest = self
            .endpoint
            .strip_prefix("http://")
            .context("only http:// ONVIF endpoints supported")?;
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        let path = format!("/{path}");
        let mut socket = TcpStream::connect(host).await?;
        let body = match &self.user {
            Some((user, password)) => security_envelope(body, user, password),
            None => body.to_string(),
        };
        let req = format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/soap+xml; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(req.as_bytes()).await?;
        let mut buf = Vec::new();
        socket.read_to_end(&mut buf).await?;
        let text = String::from_utf8_lossy(&buf);
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
            bail!("ONVIF HTTP: {}", head.lines().next().unwrap_or(""));
        }
        Ok(body.to_string())
    }

    pub async fn get_device_information(&self) -> Result<String> {
        self.call(&envelope("<tds:GetDeviceInformation/>")).await
    }

    pub async fn get_profiles(&self) -> Result<String> {
        self.call(&envelope("<trt:GetProfiles/>")).await
    }

    pub async fn get_stream_uri(&self, profile: &str) -> Result<String> {
        let body = format!(
            "<trt:GetStreamUri><trt:StreamSetup><tt:Stream>RTP-Unicast</tt:Stream><tt:Transport><tt:Protocol>RTSP</tt:Protocol></tt:Transport></trt:StreamSetup><trt:ProfileToken>{profile}</trt:ProfileToken></trt:GetStreamUri>"
        );
        let out = self.call(&envelope(&body)).await?;
        tag(&out, "<tt:Uri>", "</tt:Uri>")
            .or_else(|| tag(&out, "<trt:Uri>", "</trt:Uri>"))
            .map(str::to_string)
            .context("GetStreamUri response missing Uri")
    }

    pub async fn continuous_move(&self, x: f32, y: f32, zoom: f32) -> Result<String> {
        let body = format!(
            "<tptz:ContinuousMove><tptz:ProfileToken>profile1</tptz:ProfileToken><tptz:Velocity><tt:PanTilt x=\"{x}\" y=\"{y}\"/><tt:Zoom x=\"{zoom}\"/></tptz:Velocity></tptz:ContinuousMove>"
        );
        self.call(&envelope(&body)).await
    }

    pub async fn ptz_stop(&self) -> Result<String> {
        self.call(&envelope("<tptz:Stop><tptz:ProfileToken>profile1</tptz:ProfileToken><tptz:PanTilt>true</tptz:PanTilt><tptz:Zoom>true</tptz:Zoom></tptz:Stop>")).await
    }
}

pub fn envelope(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <s:Envelope xmlns:s=\"http://www.w3.org/2003/05/soap-envelope\" \
         xmlns:tds=\"http://www.onvif.org/ver10/device/wsdl\" \
         xmlns:trt=\"http://www.onvif.org/ver10/media/wsdl\" \
         xmlns:tptz=\"http://www.onvif.org/ver20/ptz/wsdl\" \
         xmlns:tt=\"http://www.onvif.org/ver10/schema\">\
         <s:Body>{body}</s:Body></s:Envelope>"
    )
}

fn tag<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(&text[start..end])
}

fn security_envelope(body: &str, user: &str, password: &str) -> String {
    use base64::Engine;
    let mut nonce = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut nonce).ok();
    let created = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let mut digest_input = Vec::with_capacity(nonce.len() + created.len() + password.len());
    digest_input.extend_from_slice(&nonce);
    digest_input.extend_from_slice(created.as_bytes());
    digest_input.extend_from_slice(password.as_bytes());
    let digest = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, &digest_input);
    let password_digest = base64::engine::general_purpose::STANDARD.encode(digest.as_ref());
    let nonce_b64 = base64::engine::general_purpose::STANDARD.encode(nonce);
    let user = user.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let header = format!(
        "<s:Header><wsse:Security xmlns:wsse=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd\" xmlns:wsu=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd\"><wsse:UsernameToken><wsse:Username>{user}</wsse:Username><wsse:Password Type=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-username-token-profile-1.0#PasswordDigest\">{password_digest}</wsse:Password><wsse:Nonce EncodingType=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-soap-message-security-1.0#Base64Binary\">{nonce_b64}</wsse:Nonce><wsu:Created>{created}</wsu:Created></wsse:UsernameToken></wsse:Security></s:Header>"
    );
    body.replacen("<s:Body>", &format!("{header}<s:Body>"), 1)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Discovery {
    pub endpoint: String,
    pub types: String,
    pub scopes: Vec<String>,
}

pub fn probe_message() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
     <e:Envelope xmlns:e=\"http://www.w3.org/2003/05/soap-envelope\" \
     xmlns:w=\"http://schemas.xmlsoap.org/ws/2004/08/addressing\" \
     xmlns:d=\"http://schemas.xmlsoap.org/ws/2005/04/discovery\" \
     xmlns:dn=\"http://www.onvif.org/ver10/network/wsdl\">\
     <e:Header><w:MessageID>uuid:m7s-discover</w:MessageID>\
     <w:To>urn:schemas-xmlsoap-org:ws:2005:04:discovery</w:To>\
     <w:Action>http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe</w:Action></e:Header>\
     <e:Body><d:Probe><d:Types>dn:NetworkVideoTransmitter</d:Types></d:Probe></e:Body></e:Envelope>".to_string()
}

pub fn parse_discovery(xml: &str) -> Vec<Discovery> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<d:ProbeMatches>") {
        let block = &rest[start..];
        let end = block.find("</d:ProbeMatches>").map(|e| e + 16).unwrap_or(block.len());
        let block = &block[..end];
        if let Some(endpoint) = block
            .split("<d:XAddrs>")
            .nth(1)
            .and_then(|s| s.split("</d:XAddrs>").next())
            .and_then(|s| s.split_whitespace().next())
        {
            out.push(Discovery {
                endpoint: endpoint.to_string(),
                types: tag(block, "<d:Types>", "</d:Types>").unwrap_or("").to_string(),
                scopes: tag(block, "<d:Scopes>", "</d:Scopes>")
                    .unwrap_or("")
                    .split_whitespace()
                    .map(str::to_string)
                    .collect(),
            });
        }
        rest = &block[end..];
    }
    out
}

pub async fn discover_at(target: std::net::SocketAddr, timeout: std::time::Duration) -> Result<Vec<Discovery>> {
    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await?;
    socket.send_to(probe_message().as_bytes(), target).await?;
    let mut buf = [0u8; 65535];
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return Ok(Vec::new());
        }
        let Ok(Ok((n, _))) = tokio::time::timeout(left, socket.recv_from(&mut buf)).await else {
            return Ok(Vec::new());
        };
        let text = String::from_utf8_lossy(&buf[..n]);
        let found = parse_discovery(&text);
        if !found.is_empty() {
            return Ok(found);
        }
    }
}

pub async fn discover(timeout: std::time::Duration) -> Result<Vec<Discovery>> {
    discover_at(
        std::net::SocketAddr::from(([239, 255, 255, 250], 3702)),
        timeout,
    )
    .await
}
