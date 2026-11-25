//! Optional MySQL user store (`M7S_MYSQL_URL`).

use mysql_async::prelude::*;
use std::sync::OnceLock;

fn pool() -> Option<&'static mysql_async::Pool> {
    static POOL: OnceLock<Option<mysql_async::Pool>> = OnceLock::new();
    POOL.get_or_init(|| { std::env::var("M7S_MYSQL_URL").ok().and_then(|u| mysql_async::Opts::from_url(&u).ok()).map(mysql_async::Pool::new) })
        .as_ref()
}

pub async fn check(username: &str, password: &str) -> Result<bool, String> {
    let Some(pool) = pool() else {
        return Ok(false);
    };
    let mut conn = pool.get_conn().await.map_err(|e| e.to_string())?;
    let hash: Option<String> = conn
        .exec_first(
            "SELECT hash FROM users WHERE username = ?",
            (username,),
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(hash.as_deref() == Some(crate::users::hash(password).as_str()))
}

pub async fn add(username: &str, password: &str) -> Result<(), String> {
    let Some(pool) = pool() else {
        return Err("M7S_MYSQL_URL not set".to_string());
    };
    let mut conn = pool.get_conn().await.map_err(|e| e.to_string())?;
    conn.exec_drop(
        "REPLACE INTO users (username, hash) VALUES (?, ?)",
        (username, crate::users::hash(password)),
    )
    .await
    .map_err(|e| e.to_string())
}

pub async fn count() -> Result<u64, String> {
    let Some(pool) = pool() else {
        return Ok(0);
    };
    let mut conn = pool.get_conn().await.map_err(|e| e.to_string())?;
    conn.query_first("SELECT COUNT(*) FROM users")
        .await
        .map_err(|e| e.to_string())
        .map(|v| v.unwrap_or(0))
}
