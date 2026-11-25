use m7s_rs::meta_db::MetaDb;
use m7s_sdk::MetaStore;

#[test]
fn meta_db_kv_and_migration() {
    let path = std::path::PathBuf::from(format!(
        "{}/../../target/meta-kv-{}.db",
        env!("CARGO_MANIFEST_DIR"),
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let db = MetaDb::open(&path).unwrap();
    assert_eq!(db.schema_version(), 3);
    db.put("k", "v").unwrap();
    assert_eq!(db.get("k").unwrap().as_deref(), Some("v"));
    db.put("k", "v2").unwrap();
    assert_eq!(db.get("k").unwrap().as_deref(), Some("v2"));
    db.delete("k").unwrap();
    assert_eq!(db.get("k").unwrap(), None);
    let _ = std::fs::remove_file(&path);
}
