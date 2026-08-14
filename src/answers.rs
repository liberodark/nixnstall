use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Answers {
    #[serde(default)]
    pub options: std::collections::BTreeMap<String, serde_json::Value>,
    pub device: String,
    pub layout: String,
    pub hostname: String,
    pub username: String,
    pub hashed_password: String,
    pub profile: String,
}

impl Answers {
    pub fn template_data(
        &self,
        imports: &[String],
        vars: &std::collections::BTreeMap<String, toml::Value>,
        hardware_config: bool,
    ) -> serde_json::Value {
        let vars: serde_json::Value = serde_json::to_value(vars).unwrap_or(serde_json::Value::Null);
        serde_json::json!({
            "device": self.device,
            "layout": self.layout,
            "hostname": self.hostname,
            "username": self.username,
            "hashedPassword": self.hashed_password,
            "profile": self.profile,
            "imports": imports,
            "vars": vars,
            "hardwareConfig": hardware_config,
            "opts": self.options,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Answers;

    fn sample() -> Answers {
        Answers {
            options: Default::default(),
            device: "/dev/nvme0n1".into(),
            layout: "btrfs".into(),
            hostname: "bc250".into(),
            username: "nixos".into(),
            hashed_password: "$y$j9T$x".into(),
            profile: "steam".into(),
        }
    }

    #[test]
    fn roundtrips_through_json() {
        let a = sample();
        let text = serde_json::to_string(&a).unwrap();
        let back: Answers = serde_json::from_str(&text).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn exposes_imports_to_templates() {
        let data = sample().template_data(
            &["../../extras/cache.nix".to_string()],
            &Default::default(),
            true,
        );
        assert_eq!(data["hostname"], "bc250");
        assert_eq!(data["imports"][0], "../../extras/cache.nix");
    }

    #[test]
    fn exposes_profile_vars_to_templates() {
        let mut vars = std::collections::BTreeMap::new();
        vars.insert("steam".to_string(), toml::Value::Boolean(true));
        let data = sample().template_data(&[], &vars, true);
        assert_eq!(data["vars"]["steam"], true);
    }
}
