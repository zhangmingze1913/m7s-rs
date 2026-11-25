use std::sync::Arc;
use std::time::Instant;

fn main() {
    let ring = m7s_rs::ring::RingBuffer::new(4096);
    let frames: Vec<Arc<u64>> = (0..4096).map(Arc::new).collect();
    let n = 1_000_000usize;
    let start = Instant::now();
    for i in 0..n {
        ring.push(frames[i % frames.len()].clone());
    }
    let elapsed = start.elapsed();
    let ns = elapsed.as_nanos() as f64 / n as f64;
    println!(
        "ring_push n={n} elapsed_ms={} ns_per_write={ns:.1}",
        elapsed.as_millis()
    );
}
