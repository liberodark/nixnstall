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
    #[serde(default)]
    pub question: Vec<Question>,
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
    #[serde(default = "default_true")]
    pub hardware_config: bool,
    #[serde(default)]
    pub apply_keymap_from: Option<String>,
}

fn default_true() -> bool {
    true
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
    #[serde(default)]
    pub vars: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
    Bool {
        key: String,
        prompt: String,
        #[serde(default)]
        default: bool,
        #[serde(default)]
        help: String,
    },
    Choice {
        key: String,
        prompt: String,
        choices: Vec<String>,
        #[serde(default)]
        default: Option<String>,
        #[serde(default)]
        help: String,
    },
}

impl Question {
    pub fn key(&self) -> &str {
        match self {
            Question::Bool { key, .. } | Question::Choice { key, .. } => key,
        }
    }

    pub fn prompt(&self) -> &str {
        match self {
            Question::Bool { prompt, .. } | Question::Choice { prompt, .. } => prompt,
        }
    }

    pub fn help(&self) -> &str {
        match self {
            Question::Bool { help, .. } | Question::Choice { help, .. } => help,
        }
    }

    pub fn default_value(&self) -> serde_json::Value {
        match self {
            Question::Bool { default, .. } => serde_json::Value::Bool(*default),
            Question::Choice {
                choices, default, ..
            } => serde_json::Value::String(
                default
                    .clone()
                    .or_else(|| choices.first().cloned())
                    .unwrap_or_default(),
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Render {
    pub template: PathBuf,
    pub output: String,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| NixstallError::Config {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        let cfg: Config = toml::from_str(&text).map_err(|e| NixstallError::Config {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        cfg.validate(path)?;
        Ok(cfg)
    }

    fn validate(&self, path: &Path) -> Result<()> {
        if self.layout.is_empty() {
            return Err(NixstallError::Config {
                path: path.display().to_string(),
                message: "at least one [layout.*] is required".to_string(),
            });
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
    fn keymap_question_is_optional() {
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        assert!(cfg.installer.apply_keymap_from.is_none());

        let with = SAMPLE.replace(
            "hosts_dir = \"hosts\"",
            "hosts_dir = \"hosts\"\napply_keymap_from = \"keymap\"",
        );
        let cfg: Config = toml::from_str(&with).unwrap();
        assert_eq!(cfg.installer.apply_keymap_from.as_deref(), Some("keymap"));
    }

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
        let err = cfg
            .validate(std::path::Path::new("installer.toml"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("layout"), "unexpected message: {err}");
    }
}
