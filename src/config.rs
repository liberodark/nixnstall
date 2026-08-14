use crate::error::{NixstallError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub installer: Installer,
    #[serde(default)]
    pub check: Vec<Check>,
    #[serde(default)]
    pub layout: BTreeMap<String, Layout>,
    #[serde(default)]
    pub profile: BTreeMap<String, Profile>,
    #[serde(default)]
    pub render: Vec<Render>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Installer {
    pub title: String,
    pub flake: PathBuf,
    #[serde(default = "default_target_flake")]
    pub target_flake: PathBuf,
    pub hosts_dir: PathBuf,
    #[serde(default = "default_hostname")]
    pub default_hostname: String,
    #[serde(default = "default_username")]
    pub default_username: String,
}

fn default_target_flake() -> PathBuf {
    PathBuf::from("/etc/nixos")
}
fn default_hostname() -> String {
    "nixos".to_string()
}
fn default_username() -> String {
    "nixos".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub warn_if_contains: Option<String>,
    #[serde(default)]
    pub warn_unless_contains: Option<String>,
    pub message: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layout {
    pub description: String,
    pub file: PathBuf,
    #[serde(default = "default_placeholder")]
    pub device_placeholder: String,
}

fn default_placeholder() -> String {
    "/dev/nvme0n1".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub description: String,
    #[serde(default)]
    pub imports: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Render {
    pub template: PathBuf,
    pub output: String,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let cfg: Config =
            toml::from_str(&text).map_err(|e| NixstallError::Config(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        if self.layout.is_empty() {
            return Err(NixstallError::Config(
                "at least one [layout.*] is required".to_string(),
            ));
        }
        Ok(())
    }

    pub fn layout(&self, name: &str) -> Result<&Layout> {
        self.layout
            .get(name)
            .ok_or_else(|| NixstallError::LayoutNotFound(name.to_string()))
    }

    pub fn profile(&self, name: &str) -> Result<&Profile> {
        self.profile
            .get(name)
            .ok_or_else(|| NixstallError::ProfileNotFound(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    const SAMPLE: &str = r#"
[installer]
title = "Test"
flake = "/etc/example"
hosts_dir = "hosts"

[layout.ext4]
description = "Simple"
file = "installer/disko/ext4.nix"

[profile.minimal]
description = "Console only"
"#;

    #[test]
    fn parses_minimal_config() {
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        assert_eq!(cfg.installer.target_flake.to_str().unwrap(), "/etc/nixos");
        assert_eq!(cfg.installer.default_username, "nixos");
        assert!(cfg.layout.contains_key("ext4"));
    }

    #[test]
    fn rejects_config_without_layout() {
        let text = SAMPLE.replace("[layout.ext4]", "[unused.ext4]");
        let cfg: Config = toml::from_str(&text).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("layout"), "unexpected message: {err}");
    }
}
