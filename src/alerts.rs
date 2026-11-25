//! Lightweight alert evaluation for health/overview endpoints.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Alert {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub value: u64,
    pub threshold: u64,
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

pub fn evaluate_with(
    streams: u64,
    dropped: u64,
    max_streams: u64,
    max_dropped: u64,
) -> Vec<Alert> {
    let mut out = Vec::new();
    if max_streams > 0 && streams >= max_streams {
        out.push(Alert {
            code: "streams.high".to_string(),
            severity: "warning".to_string(),
            message: format!("streams {streams} >= threshold {max_streams}"),
            value: streams,
            threshold: max_streams,
        });
    }
    if max_dropped > 0 && dropped >= max_dropped {
        out.push(Alert {
            code: "dispatch.dropped".to_string(),
            severity: "warning".to_string(),
            message: format!("dropped frames {dropped} >= threshold {max_dropped}"),
            value: dropped,
            threshold: max_dropped,
        });
    }
    out
}

pub fn evaluate(streams: u64, dropped: u64) -> Vec<Alert> {
    evaluate_with(
        streams,
        dropped,
        env_u64("M7S_ALERT_STREAMS", 9_000),
        env_u64("M7S_ALERT_DROPPED", 1_000),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds() {
        assert!(evaluate_with(10, 0, 100, 100).is_empty());
        let a = evaluate_with(100, 0, 100, 100);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].code, "streams.high");
        let b = evaluate_with(0, 100, 100, 100);
        assert_eq!(b[0].code, "dispatch.dropped");
    }
}
