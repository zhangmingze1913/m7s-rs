use m7s_rs::events;
use m7s_rs::stream::StreamManager;

#[test]
fn stream_events_publish() {
    let mut rx = events::subscribe();
    let streams = StreamManager::new(5);
    let name = format!("evt-{}", std::process::id());
    streams.get_or_create("live", &name);
    let event = rx.try_recv().expect("created event");
    assert!(event.contains("stream.created"), "{event}");
    assert!(event.contains(&name), "{event}");
    streams.remove("live", &name);
    let event = rx.try_recv().expect("removed event");
    assert!(event.contains("stream.removed"), "{event}");
}
