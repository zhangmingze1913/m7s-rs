//! Minimal blocking HTTP client injected via EngineContext.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub struct HttpClientImpl;

impl m7s_sdk::HttpClient for HttpClientImpl {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        let rest = url.strip_prefix("http://").ok_or("only http:// supported")?;
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = if host.contains(':') { host.to_string() } else { format!("{host}:80") };
        let mut stream = TcpStream::connect(&host).map_err(|e| e.to_string())?;
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
        stream
            .write_all(format!("GET /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes())
            .map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&buf);
        let (head, body) = text.split_once("\r\n\r\n").ok_or("bad http response")?;
        if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
            return Err(head.lines().next().unwrap_or("http error").to_string());
        }
        Ok(body.as_bytes().to_vec())
    }
}
