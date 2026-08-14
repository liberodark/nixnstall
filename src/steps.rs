use crate::answers::Answers;
use crate::config::Config;
use crate::error::{NixstallError, Result};
use crate::runner::{capture, stream};
use handlebars::Handlebars;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc::UnboundedSender;

pub struct Installer<'a> {
    pub config: &'a Config,
    pub answers: &'a Answers,
    pub log: UnboundedSender<String>,
    pub dry_run: bool,
}

impl<'a> Installer<'a> {
    pub async fn run(&self) -> Result<()> {
        let disko = self.render_disko()?;
        self.partition(&disko).await?;
        self.copy_flake()?;
        self.generate_host(&disko)?;
        self.install().await?;
        Ok(())
    }

    fn say(&self, msg: impl Into<String>) {
        let _ = self.log.send(msg.into());
    }

    fn render_disko(&self) -> Result<String> {
        let layout = self.config.layout(&self.answers.layout)?;
        let path = self.config.installer.flake.join(&layout.file);
        let text = std::fs::read_to_string(&path)?;
        Ok(text.replace(&layout.device_placeholder, &self.answers.device))
    }

    async fn partition(&self, disko: &str) -> Result<()> {
        let tmp = PathBuf::from("/tmp/nixstall-disko.nix");
        std::fs::write(&tmp, disko)?;
        self.say(format!(
            "Partitioning {} ({})",
            self.answers.device, self.answers.layout
        ));
        if self.dry_run {
            self.say("dry-run: skipping disko");
            return Ok(());
        }
        stream(
            "disko",
            &[
                "--mode",
                "destroy,format,mount",
                "--yes-wipe-all-disks",
                tmp.to_str().unwrap_or_default(),
            ],
            &self.log,
        )
        .await
    }

    fn target_flake(&self) -> PathBuf {
        PathBuf::from("/mnt").join(
            self.config
                .installer
                .target_flake
                .strip_prefix("/")
                .unwrap_or(&self.config.installer.target_flake),
        )
    }

    fn copy_flake(&self) -> Result<()> {
        let dest = self.target_flake();
        self.say(format!("Copying flake to {}", dest.display()));
        if self.dry_run {
            return Ok(());
        }
        std::fs::create_dir_all(&dest)?;
        copy_dir(&self.config.installer.flake, &dest)?;
        make_writable(&dest)?;
        Ok(())
    }

    fn host_dir(&self) -> PathBuf {
        self.target_flake()
            .join(&self.config.installer.hosts_dir)
            .join(&self.answers.hostname)
    }

    fn generate_host(&self, disko: &str) -> Result<()> {
        let dir = self.host_dir();
        self.say(format!("Generating {}", dir.display()));
        if self.dry_run {
            return Ok(());
        }
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("disko.nix"), disko)?;

        std::fs::write(
            dir.join("answers.json"),
            serde_json::to_string_pretty(self.answers)
                .map_err(|e| NixstallError::Template(e.to_string()))?,
        )?;

        let imports = self
            .config
            .profile(&self.answers.profile)
            .map(|p| p.imports.clone())
            .unwrap_or_default();
        let data = self.answers.template_data(&imports);

        let mut hb = Handlebars::new();
        hb.set_strict_mode(true);
        for render in &self.config.render {
            let template = self.config.installer.flake.join(&render.template);
            let text = std::fs::read_to_string(&template)?;
            let out = hb
                .render_template(&text, &data)
                .map_err(|e| NixstallError::Template(e.to_string()))?;
            std::fs::write(dir.join(&render.output), out)?;
        }
        Ok(())
    }

    async fn install(&self) -> Result<()> {
        let flake = format!(
            "{}#{}",
            self.target_flake().display(),
            self.answers.hostname
        );
        self.say(format!("Installing {flake}"));
        if self.dry_run {
            self.say("dry-run: skipping nixos-install");
            return Ok(());
        }
        stream(
            "nixos-install",
            &["--root", "/mnt", "--flake", &flake, "--no-root-password"],
            &self.log,
        )
        .await
    }
}

pub async fn hash_password(plain: &str) -> Result<String> {
    let out = capture("mkpasswd", &["-m", "yescrypt", plain]).await?;
    Ok(out.trim().to_string())
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            std::fs::create_dir_all(&target)?;
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn make_writable(path: &Path) -> Result<()> {
    for entry in walk(path)? {
        let mut perms = std::fs::metadata(&entry)?.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&entry, perms)?;
    }
    Ok(())
}

fn walk(path: &Path) -> Result<Vec<PathBuf>> {
    let mut out = vec![path.to_path_buf()];
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            out.extend(walk(&entry?.path())?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::copy_dir;
    use tempfile::tempdir;

    #[test]
    fn copies_nested_directories() {
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("a/b")).unwrap();
        std::fs::write(src.path().join("a/b/c.nix"), "{}").unwrap();

        copy_dir(src.path(), dst.path()).unwrap();
        assert!(dst.path().join("a/b/c.nix").exists());
    }
}

#[cfg(test)]
mod template_tests {
    use crate::answers::Answers;
    use handlebars::Handlebars;

    const TEMPLATE: &str = include_str!("../examples/bc-250/templates/default.nix.hbs");

    fn answers(profile: &str) -> Answers {
        Answers {
            device: "/dev/nvme0n1".into(),
            layout: "btrfs".into(),
            hostname: "bc250".into(),
            username: "joueur".into(),
            hashed_password: "$y$j9T$x".into(),
            profile: profile.into(),
        }
    }

    fn render(profile: &str) -> String {
        let a = answers(profile);
        let imports = vec!["../../extras/cache.nix".to_string()];
        let mut hb = Handlebars::new();
        hb.set_strict_mode(true);
        hb.render_template(TEMPLATE, &a.template_data(&imports))
            .expect("template renders")
    }

    #[test]
    fn renders_steam_profile() {
        let out = render("steam");
        assert!(out.contains(r#"networking.hostName = "bc250";"#));
        assert!(out.contains(r#"jovian.steam.user = "joueur";"#));
        assert!(out.contains("../../extras/cache.nix"));
    }

    #[test]
    fn minimal_profile_has_no_steam_user() {
        let out = render("minimal");
        assert!(!out.contains("jovian.steam.user"));
    }
}
