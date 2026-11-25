use m7s_rs::license::{self, Claims};

fn sign(edition: &str, seats: u32, expires: u64) -> String {
    unsafe { std::env::set_var("M7S_LICENSE_SECRET", "test-secret") };
    license::sign(&Claims {
        edition: edition.to_string(),
        expires,
        seats,
        customer: "test".to_string(),
    })
    .unwrap()
}

fn set(token: &str) {
    unsafe {
        std::env::set_var("M7S_ENFORCE_LICENSE", "1");
        std::env::set_var("M7S_LICENSE", token);
    }
}

#[test]
fn editions_and_seats() {
    let future = 4_000_000_000;
    set(&sign("free", 10, future));
    assert!(!license::premium_allowed());
    assert!(!license::seats_available(0));
    assert!(!license::cluster_allowed());

    set(&sign("pro", 10, future));
    assert!(license::premium_allowed());
    assert!(license::seats_available(9));
    assert!(!license::seats_available(10));
    assert!(!license::cluster_allowed());

    set(&sign("enterprise", 100, future));
    assert!(license::premium_allowed());
    assert!(license::seats_available(99));
    assert!(license::cluster_allowed());

    set(&sign("pro", 10, 1));
    assert!(!license::premium_allowed());
    assert!(!license::seats_available(0));

    unsafe { std::env::remove_var("M7S_LICENSE") };
    assert!(!license::premium_allowed());
    assert!(!license::seats_available(0));
    assert!(!license::cluster_allowed());

    unsafe { std::env::remove_var("M7S_ENFORCE_LICENSE") };
    assert!(license::premium_allowed());
    assert!(license::seats_available(0));
    assert!(license::cluster_allowed());
}
