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
#[serde(untagged)]
enum LsblkSize {
    Bytes(u64),
    Text(String),
}

impl LsblkSize {
    fn bytes(&self) -> u64 {
        match self {
            LsblkSize::Bytes(n) => *n,
            LsblkSize::Text(s) => s.parse().unwrap_or(0),
        }
    }
}

#[derive(Deserialize)]
struct LsblkDevice {
    name: String,
    size: LsblkSize,
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
        serde_json::from_str(&json).map_err(|e| NixstallError::DiskProbe(e.to_string()))?;

    let mut out: Vec<Disk> = parsed
        .blockdevices
        .into_iter()
        .filter(|d| d.kind == "disk" && !d.rm.unwrap_or(false))
        .map(|d| Disk {
            path: d.name,
            size: human_size(d.size.bytes()),
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
    use super::{Disk, LsblkOutput, human_size};

    #[test]
    fn formats_sizes() {
        assert_eq!(human_size(0), "0B");
        assert_eq!(human_size(1024), "1K");
        assert_eq!(human_size(500_107_862_016), "466G");
    }

    #[test]
    fn parses_numeric_sizes() {
        let json = r#"{"blockdevices":[{"name":"/dev/sda","size":500107862016,"model":"CT500","type":"disk","rm":false}]}"#;
        let parsed: LsblkOutput = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.blockdevices[0].size.bytes(), 500_107_862_016);
    }

    #[test]
    fn parses_string_sizes() {
        let json = r#"{"blockdevices":[{"name":"/dev/sda","size":"500107862016","model":null,"type":"disk"}]}"#;
        let parsed: LsblkOutput = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.blockdevices[0].size.bytes(), 500_107_862_016);
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

#[cfg(test)]
mod lsblk_regression {
    use super::LsblkOutput;

    const REAL_OUTPUT: &str = r#"{
   "blockdevices": [
      {
         "name": "/dev/sda",
         "size": 1634750464,
         "model": null,
         "type": "disk",
         "rm": true
      },{
         "name": "/dev/nvme0n1",
         "size": 500107862016,
         "model": "CT500P3SSD8",
         "type": "disk",
         "rm": false
      }
   ]
}"#;

    #[test]
    fn parses_real_lsblk_output() {
        let parsed: LsblkOutput = serde_json::from_str(REAL_OUTPUT).unwrap();
        assert_eq!(parsed.blockdevices.len(), 2);
        assert_eq!(parsed.blockdevices[0].size.bytes(), 1_634_750_464);
    }
}
