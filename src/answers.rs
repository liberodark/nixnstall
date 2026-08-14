use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Answers {
    pub device: String,
    pub layout: String,
    pub hostname: String,
    pub username: String,
    pub hashed_password: String,
    pub profile: String,
}

impl Answers {
    pub fn template_data(&self, imports: &[String]) -> serde_json::Value {
        serde_json::json!({
            "device": self.device,
            "layout": self.layout,
            "hostname": self.hostname,
            "username": self.username,
            "hashedPassword": self.hashed_password,
            "profile": self.profile,
            "imports": imports,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Answers;

    fn sample() -> Answers {
        Answers {
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
        let data = sample().template_data(&["../../extras/cache.nix".to_string()]);
        assert_eq!(data["hostname"], "bc250");
        assert_eq!(data["imports"][0], "../../extras/cache.nix");
    }
}
