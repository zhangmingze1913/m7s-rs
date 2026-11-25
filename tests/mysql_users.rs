#[test]
fn mysql_user_roundtrip() {
    if std::env::var("M7S_MYSQL_URL").is_err() {
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            m7s_rs::mysql_users::add("mysql_test", "secret")
                .await
                .unwrap();
            assert!(m7s_rs::mysql_users::check("mysql_test", "secret").await.unwrap());
            assert!(!m7s_rs::mysql_users::check("mysql_test", "bad").await.unwrap());
            assert!(m7s_rs::mysql_users::count().await.unwrap() >= 1);
        });
}
