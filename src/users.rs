//! File-backed user store (JSON, SHA-256 password hashes).

use ring::digest;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub username: String,
    pub hash: String,
    #[serde(default = "default_role")]
    pub role: String,
}

fn default_role() -> String {
    "viewer".to_string()
}

fn path() -> std::path::PathBuf {
    std::env::var("M7S_USER_DB")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("data/users.json"))
}

pub fn hash(password: &str) -> String {
    digest::digest(&digest::SHA256, password.as_bytes())
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn load_at(path: &std::path::Path) -> Vec<User> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_at(path: &std::path::Path, users: &[User]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(users).unwrap_or_default())
}

pub fn add_at(path: &std::path::Path, username: &str, password: &str) -> std::io::Result<()> {
    add_with_role_at(path, username, password, "admin")
}

pub fn add_with_role_at(path: &std::path::Path, username: &str, password: &str, role: &str) -> std::io::Result<()> {
    let mut users = load_at(path);
    users.retain(|u| u.username != username);
    users.push(User {
        username: username.to_string(),
        hash: hash(password),
        role: role.to_string(),
    });
    save_at(path, &users)
}

pub fn role_at(path: &std::path::Path, username: &str) -> Option<String> {
    load_at(path).into_iter().find(|u| u.username == username).map(|u| u.role)
}
pub fn check_at(path: &std::path::Path, username: &str, password: &str) -> bool {
    load_at(path)
        .iter()
        .any(|u| u.username == username && u.hash == hash(password))
}

pub fn add_with_role(username: &str, password: &str, role: &str) -> std::io::Result<()> {
    add_with_role_at(&path(), username, password, role)
}

pub fn role(username: &str) -> Option<String> {
    role_at(&path(), username)
}

pub fn add(username: &str, password: &str) -> std::io::Result<()> {
    add_at(&path(), username, password)
}

pub fn check(username: &str, password: &str) -> bool {
    check_at(&path(), username, password)
}

pub fn count() -> usize {
    load_at(&path()).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_check() {
        let p = std::path::PathBuf::from(format!("target/m7s-users-{}.json", std::process::id()));
        add_at(&p, "alice", "pw").unwrap();
        assert!(check_at(&p, "alice", "pw"));
        assert!(!check_at(&p, "alice", "bad"));
        let _ = std::fs::remove_file(p);
    }
}
