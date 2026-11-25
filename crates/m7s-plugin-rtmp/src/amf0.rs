//! Minimal AMF0 codec for RTMP commands.

use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq)]
pub enum Amf0 {
    Number(f64),
    Boolean(bool),
    String(String),
    Object(Vec<(String, Amf0)>),
    Null,
    Undefined,
    EcmaArray(Vec<(String, Amf0)>),
    StrictArray(Vec<Amf0>),
}

impl Amf0 {
    pub fn get(&self, key: &str) -> Option<&Amf0> {
        match self {
            Amf0::Object(items) | Amf0::EcmaArray(items) => {
                items.iter().find(|(k, _)| k == key).map(|(_, v)| v)
            }
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Amf0::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Amf0::Number(n) => Some(*n),
            _ => None,
        }
    }
}

pub fn decode_all(data: &[u8]) -> Result<Vec<Amf0>> {
    let mut pos = 0;
    let mut out = Vec::new();
    while pos < data.len() {
        out.push(decode(data, &mut pos)?);
    }
    Ok(out)
}

pub fn decode(data: &[u8], pos: &mut usize) -> Result<Amf0> {
    let marker = *data.get(*pos).ok_or_else(|| anyhow::anyhow!("AMF0 eof"))?;
    *pos += 1;
    match marker {
        0x00 => {
            if *pos + 8 > data.len() {
                bail!("AMF0 number truncated");
            }
            let n = f64::from_be_bytes(data[*pos..*pos + 8].try_into().unwrap());
            *pos += 8;
            Ok(Amf0::Number(n))
        }
        0x01 => {
            let b = *data
                .get(*pos)
                .ok_or_else(|| anyhow::anyhow!("AMF0 bool eof"))?;
            *pos += 1;
            Ok(Amf0::Boolean(b != 0))
        }
        0x02 => {
            let len = read_u16(data, pos)? as usize;
            let s = read_string(data, pos, len)?;
            Ok(Amf0::String(s))
        }
        0x03 => Ok(Amf0::Object(read_properties(data, pos)?)),
        0x05 | 0x06 => Ok(Amf0::Null),
        0x08 => {
            let _ = read_u32(data, pos)?;
            Ok(Amf0::EcmaArray(read_properties(data, pos)?))
        }
        0x09 => bail!("unexpected AMF0 object end marker"),
        0x0a => {
            let len = read_u32(data, pos)? as usize;
            let mut out = Vec::with_capacity(len);
            for _ in 0..len {
                out.push(decode(data, pos)?);
            }
            Ok(Amf0::StrictArray(out))
        }
        0x0b => {
            let _ = read_u32(data, pos)?;
            let _ = read_u32(data, pos)?;
            let _ = read_u16(data, pos)?;
            Ok(Amf0::Null)
        }
        0x0c => {
            let len = read_u32(data, pos)? as usize;
            let s = read_string(data, pos, len)?;
            Ok(Amf0::String(s))
        }
        _ => bail!("unsupported AMF0 marker 0x{marker:02x}"),
    }
}

fn read_properties(data: &[u8], pos: &mut usize) -> Result<Vec<(String, Amf0)>> {
    let mut out = Vec::new();
    loop {
        let len = read_u16(data, pos)? as usize;
        if len == 0 {
            // Object end: 00 00 09.
            if *pos < data.len() {
                *pos += 1;
            }
            break;
        }
        let key = read_string(data, pos, len)?;
        let value = decode(data, pos)?;
        out.push((key, value));
    }
    Ok(out)
}

fn read_u16(data: &[u8], pos: &mut usize) -> Result<u16> {
    if *pos + 2 > data.len() {
        bail!("AMF0 u16 truncated");
    }
    let v = u16::from_be_bytes([data[*pos], data[*pos + 1]]);
    *pos += 2;
    Ok(v)
}

fn read_u32(data: &[u8], pos: &mut usize) -> Result<u32> {
    if *pos + 4 > data.len() {
        bail!("AMF0 u32 truncated");
    }
    let v = u32::from_be_bytes(data[*pos..*pos + 4].try_into().unwrap());
    *pos += 4;
    Ok(v)
}

fn read_string(data: &[u8], pos: &mut usize, len: usize) -> Result<String> {
    if *pos + len > data.len() {
        bail!("AMF0 string truncated");
    }
    let s = String::from_utf8_lossy(&data[*pos..*pos + len]).to_string();
    *pos += len;
    Ok(s)
}

pub fn encode(value: &Amf0) -> Vec<u8> {
    let mut out = Vec::new();
    encode_into(value, &mut out);
    out
}

fn encode_into(value: &Amf0, out: &mut Vec<u8>) {
    match value {
        Amf0::Number(n) => {
            out.push(0x00);
            out.extend_from_slice(&n.to_be_bytes());
        }
        Amf0::Boolean(b) => {
            out.push(0x01);
            out.push(*b as u8);
        }
        Amf0::String(s) => {
            out.push(0x02);
            out.extend_from_slice(&(s.len() as u16).to_be_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        Amf0::Object(items) => {
            out.push(0x03);
            encode_props(items, out);
        }
        Amf0::Null | Amf0::Undefined => out.push(0x05),
        Amf0::EcmaArray(items) => {
            out.push(0x08);
            out.extend_from_slice(&(items.len() as u32).to_be_bytes());
            encode_props(items, out);
        }
        Amf0::StrictArray(items) => {
            out.push(0x0a);
            out.extend_from_slice(&(items.len() as u32).to_be_bytes());
            for item in items {
                encode_into(item, out);
            }
        }
    }
}

fn encode_props(items: &[(String, Amf0)], out: &mut Vec<u8>) {
    for (key, value) in items {
        out.extend_from_slice(&(key.len() as u16).to_be_bytes());
        out.extend_from_slice(key.as_bytes());
        encode_into(value, out);
    }
    out.extend_from_slice(&[0, 0, 0x09]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_roundtrip() {
        let v = Amf0::Object(vec![
            ("app".to_string(), Amf0::String("live".to_string())),
            (
                "tcUrl".to_string(),
                Amf0::String("rtmp://x/live".to_string()),
            ),
        ]);
        let bytes = encode(&v);
        assert_eq!(decode_all(&bytes).unwrap()[0], v);
    }
}
