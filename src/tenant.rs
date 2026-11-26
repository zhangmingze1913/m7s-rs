//! Basic multi-tenant registry: token isolation and per-tenant viewer quota.
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Tenant {
    pub name: String,
    #[serde(skip_serializing)]
    pub token: String,
    pub max_streams: usize,
    pub max_viewers: usize,
    pub created: u64,
}

fn tenants() -> &'static RwLock<HashMap<String, Tenant>> {
    static TENANTS: OnceLock<RwLock<HashMap<String, Tenant>>> = OnceLock::new();
    TENANTS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn create(name: &str, token: &str, max_streams: usize, max_viewers: usize) -> bool {
    if name.is_empty() || token.is_empty() || name.contains('/') {
        return false;
    }
    let mut map = tenants().write().unwrap_or_else(|e| e.into_inner());
    if map.contains_key(name) || map.values().any(|t| t.token == token) {
        return false;
    }
    map.insert(
        name.to_string(),
        Tenant {
            name: name.to_string(),
            token: token.to_string(),
            max_streams: max_streams.max(1),
            max_viewers: max_viewers.max(1),
            created: crate::time::now_ms().max(0) as u64 / 1000,
        },
    );
    true
}

pub fn remove(name: &str) -> bool {
    tenants()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .remove(name)
        .is_some()
}

pub fn list() -> Vec<Tenant> {
    let mut out: Vec<Tenant> = tenants()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .cloned()
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn by_token(token: &str) -> Option<Tenant> {
    tenants()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .find(|t| t.token == token)
        .cloned()
}

pub fn get(name: &str) -> Option<Tenant> {
    tenants()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(name)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_find_and_remove() {
        let name = format!("t{}", std::process::id());
        assert!(create(&name, "tok-tenant", 3, 100));
        assert!(by_token("tok-tenant").is_some());
        assert!(!create(&name, "tok-other", 1, 1));
        assert!(list().iter().any(|t| t.name == name));
        assert!(remove(&name));
        assert!(by_token("tok-tenant").is_none());
    }
}
