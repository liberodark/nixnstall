use crate::config::Check;
use crate::error::{NixstallError, Result};
use crate::runner::capture;
use serde::Deserialize;
use tracing::warn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disk {
    pub path: String,
    pub size: String,
    pub model: String,
}

impl Disk {
    pub fn label(&self) -> String {
        let model = if self.model.is_empty() {
            "unknown".to_string()
        } else {
            self.model.clone()
        };
        format!("{}  {}  {}", self.path, self.size, model)
    }
}

#[derive(Deserialize)]
struct LsblkOutput {
    blockdevices: Vec<LsblkDevice>,
}

#[derive(Deserialize)]
struct LsblkDevice {
    name: String,
    size: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    rm: Option<bool>,
}

pub async fn disks() -> Result<Vec<Disk>> {
    let json = capture(
        "lsblk",
        &["-J", "-b", "-o", "NAME,SIZE,MODEL,TYPE,RM", "-p"],
    )
    .await;
    let json = match json {
        Ok(j) => j,
        Err(e) => {
            warn!("lsblk failed: {e}");
            return Err(NixstallError::NoDisk);
        }
    };

    let parsed: LsblkOutput =
        serde_json::from_str(&json).map_err(|e| NixstallError::Config(e.to_string()))?;

    let mut out: Vec<Disk> = parsed
        .blockdevices
        .into_iter()
        .filter(|d| d.kind == "disk" && !d.rm.unwrap_or(false))
        .map(|d| Disk {
            path: d.name,
            size: human_size(d.size.parse::<u64>().unwrap_or(0)),
            model: d.model.unwrap_or_default().trim().to_string(),
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));

    if out.is_empty() {
        return Err(NixstallError::NoDisk);
    }
    Ok(out)
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.0}{}", UNITS[unit])
}

pub fn ensure_disk(disks: &[Disk], device: &str) -> Result<()> {
    if disks.iter().any(|d| d.path == device) {
        Ok(())
    } else {
        Err(NixstallError::DiskNotFound(device.to_string()))
    }
}

pub async fn preflight(checks: &[Check]) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    for check in checks {
        let args: Vec<&str> = check.args.iter().map(String::as_str).collect();
        let output = capture(&check.command, &args).await.unwrap_or_default();

        let fired = match (&check.warn_if_contains, &check.warn_unless_contains) {
            (Some(needle), _) => output.contains(needle),
            (_, Some(needle)) => !output.contains(needle),
            _ => false,
        };
        if fired {
            if check.required {
                return Err(NixstallError::Preflight(check.message.clone()));
            }
            warn!(check = check.name, "preflight warning");
            warnings.push(check.message.clone());
        }
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::{Disk, human_size};

    #[test]
    fn formats_sizes() {
        assert_eq!(human_size(0), "0B");
        assert_eq!(human_size(1024), "1K");
        assert_eq!(human_size(500_107_862_016), "466G");
    }

    #[test]
    fn labels_unknown_model() {
        let d = Disk {
            path: "/dev/sda".into(),
            size: "466G".into(),
            model: String::new(),
        };
        assert!(d.label().contains("unknown"));
    }
}
