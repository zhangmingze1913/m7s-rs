#[test]
fn gifts_are_persisted() {
    let path = std::path::PathBuf::from(format!(
        "{}/../../target/gifts-{}.json",
        env!("CARGO_MANIFEST_DIR"),
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let events = path.with_extension("events.jsonl");
    let _ = std::fs::remove_file(&events);
    unsafe {
        std::env::set_var("M7S_GIFT_FILE", &path);
        std::env::set_var("M7S_GIFT_EVENT_LOG", &events);
        std::env::set_var("M7S_GIFT_EVENT_MAX", "2");
    }
    m7s_plugin_live::send_gift("persist-room", "alice", 7, 3);
    m7s_plugin_live::send_gift("persist-room", "bob", 7, 1);
    m7s_plugin_live::send_gift("persist-room", "carol", 7, 1);
    let log = std::fs::read_to_string(&events).unwrap();
    assert!(log.lines().count() <= 2, "{log}");
    assert!(log.contains("carol"), "{log}");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("persist-room"), "{text}");
    assert!(text.contains("alice"), "{text}");
    assert_eq!(m7s_plugin_live::gift_total("persist-room"), 5);
    let _ = std::fs::remove_file(&path);
}
