//! SEI NAL unit detection / extraction / insertion (H.264).

/// Build an Annex B `user_data_unregistered` SEI NAL for a 16-byte UUID.
pub fn build_sei(uuid: [u8; 16], payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(16 + payload.len());
    body.extend_from_slice(&uuid);
    body.extend_from_slice(payload);
    let mut nal = vec![0, 0, 0, 1, 0x06, 0x05];
    let mut len = body.len();
    while len >= 255 {
        nal.push(255);
        len -= 255;
    }
    nal.push(len as u8);
    nal.extend_from_slice(&body);
    nal.push(0x80);
    nal
}

/// Parse a `user_data_unregistered` SEI NAL (with or without Annex B prefix).
pub fn parse_sei(nal: &[u8]) -> Option<([u8; 16], Vec<u8>)> {
    let mut i = 0;
    if nal.starts_with(&[0, 0, 0, 1]) {
        i = 4;
    } else if nal.starts_with(&[0, 0, 1]) {
        i = 3;
    }
    if *nal.get(i)? & 0x1f != 6 {
        return None;
    }
    i += 1;
    if *nal.get(i)? != 5 {
        return None;
    }
    i += 1;
    let mut len = 0usize;
    loop {
        let b = *nal.get(i)?;
        i += 1;
        len += b as usize;
        if b != 255 {
            break;
        }
    }
    if nal.len() < i + len {
        return None;
    }
    let uuid: [u8; 16] = nal[i..i + 16].try_into().ok()?;
    Some((uuid, nal[i + 16..i + len].to_vec()))
}

/// Insert an SEI NAL before the existing access unit.
pub fn insert_sei(annexb: &[u8], uuid: [u8; 16], payload: &[u8]) -> Vec<u8> {
    let mut out = build_sei(uuid, payload);
    out.extend_from_slice(annexb);
    out
}

pub fn is_sei(nal: &[u8]) -> bool { nal.first().is_some_and(|b| b & 0x1f == 6) }

pub fn find_sei(annexb: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new(); let mut i = 0;
    while i + 3 < annexb.len() {
        if annexb[i] == 0 && annexb[i+1] == 0 && annexb[i+2] == 1 {
            let start = i + 3; let mut end = annexb.len(); let mut j = start;
            while j + 3 < annexb.len() {
                if annexb[j] == 0 && annexb[j+1] == 0 && annexb[j+2] == 1 { end = j; break; }
                j += 1;
            }
            if start < end && is_sei(&annexb[start..end]) { out.push(&annexb[start..end]); }
            i = end;
        } else { i += 1; }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_sei() {
        let b = [0, 0, 1, 6, 5, 1, 2, 0, 0, 1, 5, 1];
        assert_eq!(find_sei(&b).len(), 1);
    }

    #[test]
    fn build_and_parse_roundtrip() {
        let uuid = [7u8; 16];
        let payload = b"m7s-sei-payload";
        let sei = build_sei(uuid, payload);
        let (got_uuid, got_payload) = parse_sei(&sei).expect("parse sei");
        assert_eq!(got_uuid, uuid);
        assert_eq!(got_payload, payload);
        let frame = [0u8, 0, 1, 0x65, 9, 9];
        let out = insert_sei(&frame, uuid, payload);
        let found = find_sei(&out);
        assert_eq!(found.len(), 1);
        let (_, p) = parse_sei(found[0]).unwrap();
        assert_eq!(p, payload);
    }
}
