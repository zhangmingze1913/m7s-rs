use m7s_rs::license::{self, Claims};

#[test]
fn activate_and_revoke_license_file() {
    let dir = std::env::temp_dir().join(format!("m7s-lic-{}", std::process::id()));
    let file = dir.join("license.token");
    unsafe {
        std::env::set_var("M7S_LICENSE_SECRET", "secret");
        std::env::set_var("M7S_LICENSE_FILE", &file);
        std::env::set_var("M7S_ENFORCE_LICENSE", "1");
        std::env::remove_var("M7S_LICENSE");
    }
    let claims = Claims {
        edition: "pro".to_string(),
        expires: 4_000_000_000,
        seats: 5,
        customer: "test".to_string(),
    };
    let token = license::sign(&claims).unwrap();
    let activated = license::activate(&token).unwrap();
    assert_eq!(activated.edition, "pro");
    assert!(file.exists());
    assert!(license::premium_allowed());
    assert_eq!(license::status(0)["edition"], "pro");
    assert!(license::seats_available(4));

    license::revoke();
    assert!(!file.exists());
    assert!(!license::premium_allowed());
    assert!(license::activate_at(&file, "bad").is_err());
}
