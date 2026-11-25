use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

fn be16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}
fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn analyze(stream: &[u8]) {
    let mut i = 0;
    let mut frames = 0usize;
    let mut chans: BTreeMap<u8, usize> = BTreeMap::new();
    let mut rtp: BTreeMap<(u8, u8), (usize, u16, u32, u32)> = BTreeMap::new();
    let mut last: BTreeMap<(u8, u8), u16> = BTreeMap::new();
    let mut gaps = 0usize;
    while i + 4 <= stream.len() {
        if stream[i] != b'$' {
            i += 1;
            continue;
        }
        let ch = stream[i + 1];
        let len = be16(stream, i + 2) as usize;
        if i + 4 + len > stream.len() {
            break;
        }
        let pkt = &stream[i + 4..i + 4 + len];
        frames += 1;
        *chans.entry(ch).or_default() += 1;
        if pkt.len() >= 12 && (pkt[0] >> 6) == 2 && pkt[1] != 200 && pkt[1] != 201 {
            let pt = pkt[1] & 0x7f;
            let seq = be16(pkt, 2);
            let ts = be32(pkt, 4);
            let e = rtp.entry((ch, pt)).or_insert((0, seq, ts, 0));
            e.0 += 1;
            e.3 = ts;
            if let Some(prev) = last.insert((ch, pt), seq) {
                if seq != prev.wrapping_add(1) {
                    gaps += 1;
                }
            }
        }
        i += 4 + len;
    }
    println!("frames={frames} chans={chans:?} gaps={gaps}");
    for (k, v) in rtp {
        println!(
            "rtp {k:?} count={} first_seq={} first_ts={} last_ts={}",
            v.0, v.1, v.2, v.3
        );
    }
}

fn main() {
    let listener = TcpListener::bind("127.0.0.1:8555").unwrap();
    println!("proxy 8555 -> 8554");
    for client in listener.incoming() {
        let mut client = client.unwrap();
        let mut upstream = TcpStream::connect("127.0.0.1:8554").unwrap();
        let mut c2 = client.try_clone().unwrap();
        let mut u2 = upstream.try_clone().unwrap();
        thread::spawn(move || {
            let _ = std::io::copy(&mut c2, &mut u2);
        });
        let mut data = Vec::new();
        let mut buf = [0u8; 16384];
        loop {
            match upstream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if client.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    data.extend_from_slice(&buf[..n]);
                }
            }
        }
        println!("closed, {} bytes", data.len());
        analyze(&data);
    }
}
