//! Configuration framework: typed TOML loading with unified errors.

use std::path::Path;

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;

pub fn load<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read config {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parse config {}", path.display()))
}

pub fn from_str<T: DeserializeOwned>(text: &str) -> Result<T> {
    toml::from_str(text).context("parse config")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Demo {
        name: String,
        port: u16,
    }

    #[test]
    fn parse_demo() {
        let c: Demo = from_str("name = \"x\"\nport = 8080\n").unwrap();
        assert_eq!(c.name, "x");
        assert_eq!(c.port, 8080);
    }
}
