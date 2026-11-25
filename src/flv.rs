//! FLV tag model and parser, re-exported from `m7s-plugin-flv`.
use std::sync::Arc;
pub use m7s_plugin_flv::{FlvReader, FlvTag, flv_header};
pub type SharedTag = Arc<FlvTag>;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tag_roundtrip() {
        let tag = FlvTag::new(9, 1234, vec![0x17, 1, 2, 3]);
        let bytes = tag.to_bytes();
        let mut reader = FlvReader::new();
        reader.push(&flv_header(false, true));
        reader.push(&bytes);
        let parsed = reader.next_tag().unwrap().unwrap();
        assert_eq!(parsed.timestamp, 1234);
    }
}
