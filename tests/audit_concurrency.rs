use m7s_rs::audit::{AuditEntry, append_at, recent_at};

#[test]
fn concurrent_appends_keep_file_valid() {
    unsafe {
        std::env::set_var("M7S_AUDIT_MAX", "50");
        std::env::set_var("M7S_AUDIT_ARCHIVE", "0");
    }
    let path = std::path::PathBuf::from(format!(
        "{}/../../target/audit-conc-{}.log",
        env!("CARGO_MANIFEST_DIR"),
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let mut handles = Vec::new();
    for t in 0..10 {
        let path = path.clone();
        handles.push(std::thread::spawn(move || {
            for i in 0..10 {
                let e = AuditEntry {
                    ts: (t * 10 + i) as u64,
                    actor: "a".into(),
                    role: "admin".into(),
                    action: format!("e{t}-{i}"),
                    target: "t".into(),
                    ok: true,
                };
                append_at(&path, &e).unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() <= 50, "{}", lines.len());
    for line in &lines {
        serde_json::from_str::<AuditEntry>(line).unwrap();
    }
    assert_eq!(recent_at(&path, 100).len(), lines.len());
    assert!(!std::path::PathBuf::from(format!("{}.lock", path.display())).exists());
    let _ = std::fs::remove_file(&path);
    unsafe {
        std::env::remove_var("M7S_AUDIT_MAX");
        std::env::remove_var("M7S_AUDIT_ARCHIVE");
    }
}
