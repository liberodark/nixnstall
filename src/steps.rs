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
    pub async fn run(self) -> Result<()> {
        let disko = self.render_disko()?;
        self.partition(&disko).await?;
        self.copy_flake()?;
        self.generate_host(&disko)?;
        self.detect_hardware().await?;
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
        let tmp = PathBuf::from("/tmp/nixnstall-disko.nix");
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

        let profile = self.config.profile(&self.answers.profile)?;
        let data = self.answers.template_data(
            &profile.imports,
            &profile.vars,
            self.config.installer.hardware_config,
        );

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

    async fn detect_hardware(&self) -> Result<()> {
        if !self.config.installer.hardware_config {
            return Ok(());
        }
        self.say("Detecting hardware");
        if self.dry_run {
            return Ok(());
        }

        stream(
            "nixos-generate-config",
            &["--no-filesystems", "--root", "/mnt"],
            &self.log,
        )
        .await?;

        let generated = PathBuf::from("/mnt/etc/nixos/hardware-configuration.nix");
        std::fs::copy(
            &generated,
            self.host_dir().join("hardware-configuration.nix"),
        )?;
        let _ = std::fs::remove_file(&generated);
        let _ = std::fs::remove_file("/mnt/etc/nixos/configuration.nix");
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

    fn options(
        cu40: bool,
        auto_login: bool,
    ) -> std::collections::BTreeMap<String, serde_json::Value> {
        let mut o = std::collections::BTreeMap::new();
        o.insert("cu40".into(), serde_json::Value::Bool(cu40));
        o.insert("autoLogin".into(), serde_json::Value::Bool(auto_login));
        o.insert("steamAutoStart".into(), serde_json::Value::Bool(true));
        o.insert("videoDriver".into(), serde_json::json!("amdgpu"));
        o.insert("keymap".into(), serde_json::json!("fr"));
        o.insert("shell".into(), serde_json::json!("fish"));
        o.insert("zram".into(), serde_json::json!("50"));
        o.insert("browser".into(), serde_json::json!("firefox"));
        o.insert("locale".into(), serde_json::json!("fr_FR.UTF-8"));
        o.insert("timezone".into(), serde_json::json!("Europe/Paris"));
        o.insert("sshd".into(), serde_json::Value::Bool(true));
        o.insert("firewall".into(), serde_json::Value::Bool(false));
        o.insert("gpuClock".into(), serde_json::json!("2230"));
        o.insert("decky".into(), serde_json::Value::Bool(true));
        o.insert("gamescope".into(), serde_json::Value::Bool(true));
        o.insert("bigPicture".into(), serde_json::Value::Bool(false));
        o
    }

    fn answers(profile: &str) -> Answers {
        Answers {
            options: options(true, true),
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
        let mut session = std::collections::BTreeMap::new();
        session.insert(
            "session".to_string(),
            toml::Value::String(profile.to_string()),
        );
        let mut hb = Handlebars::new();
        hb.set_strict_mode(true);
        hb.render_template(TEMPLATE, &a.template_data(&imports, &session, true))
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
    fn options_drive_the_generated_config() {
        let out = render("plasma");
        assert!(out.contains(r#"videoDrivers = [ "amdgpu" ]"#), "{out}");
        assert!(out.contains(r#"console.keyMap = "fr""#));
        assert!(out.contains("bc250_cc_write_mode=3"));
        // gamescope is on in this fixture: Jovian owns autologin and the DM
        assert!(!out.contains("autoLogin.enable"), "{out}");
        assert!(!out.contains("sddm.enable"), "{out}");
        assert!(out.contains("jovian.steam.autoStart = true;"));
        assert!(out.contains("jovian.decky-loader.user"));
        assert!(out.contains("extras/decky.nix"), "{out}");
        assert!(out.contains(r#"desktopSession = "plasma""#));
    }

    #[test]
    fn overclock_extends_the_safe_points() {
        let out = render("plasma");
        assert!(out.contains("frequency-range.max = 2230;"), "{out}");
        assert!(out.contains("frequency = 2230; voltage = 1060;"), "{out}");
        assert!(out.contains("frequency = 2100; voltage = 1000;"), "{out}");
        // never below the 700 mV floor that re-locks the GPU to 1500 MHz
        assert!(!out.contains("voltage = 6"), "{out}");
        assert!(out.contains("services.openssh.enable = true;"));
        assert!(out.contains("networking.firewall.enable = false;"));
    }

    #[test]
    fn every_clock_step_produces_a_matching_curve() {
        for (clock, expected_top) in [
            ("1600", "frequency = 1600; voltage = 910;"),
            ("1700", "frequency = 1700; voltage = 920;"),
            ("1800", "frequency = 1800; voltage = 925;"),
            ("1850", "frequency = 1850; voltage = 930;"),
            ("1900", "frequency = 1900; voltage = 940;"),
            ("2000", "frequency = 2000; voltage = 960;"),
            ("2100", "frequency = 2100; voltage = 1000;"),
            ("2230", "frequency = 2230; voltage = 1060;"),
            ("2300", "frequency = 2300; voltage = 1075;"),
        ] {
            let mut a = answers("plasma");
            a.options
                .insert("gpuClock".into(), serde_json::json!(clock));
            let mut hb = Handlebars::new();
            hb.set_strict_mode(true);
            let out = hb
                .render_template(TEMPLATE, &a.template_data(&[], &Default::default(), true))
                .expect("template renders");

            assert!(
                out.contains(&format!("frequency-range.max = {clock};")),
                "{clock}: {out}"
            );
            // the curve must reach the ceiling, or the governor clamps below it
            assert!(out.contains(expected_top), "{clock}: {out}");
            // and must not offer a point the ceiling does not allow
            assert!(!out.contains("frequency = 2400"), "{clock}: {out}");
        }
    }

    #[test]
    fn stock_clock_leaves_the_governor_alone() {
        let mut a = answers("plasma");
        a.options
            .insert("gpuClock".into(), serde_json::json!("1500"));
        let mut hb = Handlebars::new();
        hb.set_strict_mode(true);
        let out = hb
            .render_template(TEMPLATE, &a.template_data(&[], &Default::default(), true))
            .expect("template renders");
        assert!(!out.contains("frequency-range.max"), "{out}");
    }

    #[test]
    fn locale_and_timezone_are_applied() {
        let out = render("plasma");
        assert!(
            out.contains(r#"i18n.defaultLocale = "fr_FR.UTF-8""#),
            "{out}"
        );
        assert!(out.contains(r#"LC_TIME = "fr_FR.UTF-8""#));
        assert!(out.contains(r#"time.timeZone = "Europe/Paris""#));
        assert!(out.contains(r#"xkb.layout = "fr""#));
    }

    #[test]
    fn decky_does_not_depend_on_gamescope() {
        let mut a = answers("plasma");
        a.options
            .insert("decky".into(), serde_json::Value::Bool(true));
        a.options
            .insert("gamescope".into(), serde_json::Value::Bool(false));
        let mut hb = Handlebars::new();
        hb.set_strict_mode(true);
        let out = hb
            .render_template(TEMPLATE, &a.template_data(&[], &Default::default(), true))
            .expect("template renders");
        assert!(out.contains("extras/decky.nix"), "{out}");
        assert!(out.contains("jovian.decky-loader.user"), "{out}");
        assert!(!out.contains("jovian.steam.user"), "{out}");
    }

    #[test]
    fn layout_pulls_its_filesystem_support() {
        for (layout, expected) in [
            ("bcachefs", "boot.supportedFilesystems.bcachefs = true;"),
            ("f2fs", "boot.supportedFilesystems.f2fs = true;"),
        ] {
            let mut a = answers("plasma");
            a.layout = layout.into();
            let mut hb = Handlebars::new();
            hb.set_strict_mode(true);
            let out = hb
                .render_template(TEMPLATE, &a.template_data(&[], &Default::default(), true))
                .expect("template renders");
            assert!(out.contains(expected), "{layout}: {out}");
            assert!(!out.contains("btrfs.autoScrub"), "{layout}: {out}");
        }
    }

    #[test]
    fn zram_choice_is_applied() {
        // 50 % of the 16 GB shared with the GPU is what SteamOS uses
        assert!(render("plasma").contains("zramSwap.memoryPercent = 50;"));

        let mut a = answers("plasma");
        a.options.insert("zram".into(), serde_json::json!("off"));
        let mut hb = Handlebars::new();
        hb.set_strict_mode(true);
        let out = hb
            .render_template(TEMPLATE, &a.template_data(&[], &Default::default(), true))
            .expect("template renders");
        assert!(out.contains("zramSwap.enable = false;"), "{out}");
        assert!(!out.contains("memoryPercent"), "{out}");
    }

    #[test]
    fn browser_choice_is_applied() {
        let out = render("plasma");
        assert!(out.contains("programs.firefox.enable = true;"), "{out}");
        assert!(!out.contains("pkgs.firefox"), "{out}");

        let mut a = answers("plasma");
        a.options
            .insert("browser".into(), serde_json::json!("brave"));
        let mut hb = Handlebars::new();
        hb.set_strict_mode(true);
        let out = hb
            .render_template(TEMPLATE, &a.template_data(&[], &Default::default(), true))
            .expect("template renders");
        assert!(out.contains("pkgs.brave"), "{out}");
        assert!(!out.contains("programs.firefox"), "{out}");
    }

    #[test]
    fn shell_is_applied_and_enabled() {
        let out = render("plasma");
        assert!(out.contains("shell = pkgs.fish;"), "{out}");
        assert!(out.contains("programs.fish.enable = true;"));
    }

    #[test]
    fn steam_autostart_can_be_turned_off() {
        let mut a = answers("plasma");
        a.options
            .insert("steamAutoStart".into(), serde_json::Value::Bool(false));
        let mut hb = Handlebars::new();
        hb.set_strict_mode(true);
        let out = hb
            .render_template(
                TEMPLATE,
                &a.template_data(
                    &[],
                    &{
                        let mut v = std::collections::BTreeMap::new();
                        v.insert("steam".to_string(), toml::Value::Boolean(true));
                        v
                    },
                    true,
                ),
            )
            .expect("template renders");
        assert!(out.contains("jovian.steam.autoStart = false;"), "{out}");
    }

    #[test]
    fn scrub_only_on_btrfs() {
        assert!(render("plasma").contains("btrfs.autoScrub"));

        for layout in ["ext4", "xfs", "f2fs", "bcachefs"] {
            let mut a = answers("minimal");
            a.layout = layout.into();
            let mut hb = Handlebars::new();
            hb.set_strict_mode(true);
            let out = hb
                .render_template(TEMPLATE, &a.template_data(&[], &Default::default(), true))
                .expect("template renders");
            assert!(!out.contains("btrfs.autoScrub"), "{layout}: {out}");
        }
    }

    #[test]
    fn gamescope_session_is_opt_in() {
        let mut a = answers("plasma");
        a.options
            .insert("gamescope".into(), serde_json::Value::Bool(false));
        let mut vars = std::collections::BTreeMap::new();
        vars.insert("session".to_string(), toml::Value::String("plasma".into()));
        vars.insert(
            "displayManager".to_string(),
            toml::Value::String("services.displayManager.sddm".into()),
        );
        let mut hb = Handlebars::new();
        hb.set_strict_mode(true);
        let out = hb
            .render_template(TEMPLATE, &a.template_data(&[], &vars, true))
            .expect("template renders");
        assert!(!out.contains("gamescope-session.nix"), "{out}");
        assert!(!out.contains("jovian.steam.user"), "{out}");
        // without gamescope the desktop provides its own display manager
        assert!(
            out.contains("services.displayManager.sddm.enable = true;"),
            "{out}"
        );

        a.options
            .insert("bigPicture".into(), serde_json::Value::Bool(true));
        let out = hb
            .render_template(TEMPLATE, &a.template_data(&[], &vars, true))
            .expect("template renders");
        assert!(out.contains("steam-bigpicture.nix"), "{out}");
    }
}

#[cfg(test)]
mod hardware_template_tests {
    use crate::answers::Answers;
    use handlebars::Handlebars;

    const TEMPLATE: &str = include_str!("../examples/bc-250/templates/default.nix.hbs");

    fn options() -> std::collections::BTreeMap<String, serde_json::Value> {
        let mut o = std::collections::BTreeMap::new();
        o.insert("cu40".into(), serde_json::Value::Bool(false));
        o.insert("autoLogin".into(), serde_json::Value::Bool(false));
        o.insert("steamAutoStart".into(), serde_json::Value::Bool(false));
        o.insert("videoDriver".into(), serde_json::json!("modesetting"));
        o.insert("keymap".into(), serde_json::json!("us"));
        o.insert("shell".into(), serde_json::json!("bash"));
        o.insert("zram".into(), serde_json::json!("off"));
        o.insert("browser".into(), serde_json::json!("brave"));
        o.insert("locale".into(), serde_json::json!("en_US.UTF-8"));
        o.insert("timezone".into(), serde_json::json!("UTC"));
        o.insert("sshd".into(), serde_json::Value::Bool(false));
        o.insert("firewall".into(), serde_json::Value::Bool(true));
        o.insert("gpuClock".into(), serde_json::json!("1500"));
        o.insert("decky".into(), serde_json::Value::Bool(false));
        o.insert("gamescope".into(), serde_json::Value::Bool(false));
        o.insert("bigPicture".into(), serde_json::Value::Bool(true));
        o
    }

    fn render(hardware_config: bool) -> String {
        let a = Answers {
            options: options(),
            device: "/dev/vda".into(),
            layout: "btrfs".into(),
            hostname: "bc250".into(),
            username: "nixos".into(),
            hashed_password: "$y$j9T$x".into(),
            profile: "minimal".into(),
        };
        let mut hb = Handlebars::new();
        hb.set_strict_mode(true);
        hb.render_template(
            TEMPLATE,
            &a.template_data(&[], &Default::default(), hardware_config),
        )
        .expect("template renders")
    }

    #[test]
    fn imports_generated_hardware_config() {
        assert!(render(true).contains("./hardware-configuration.nix"));
    }

    #[test]
    fn omits_it_when_disabled() {
        assert!(!render(false).contains("./hardware-configuration.nix"));
    }
}

#[cfg(test)]
mod progress_channel {
    use crate::runner::stream;

    /// The progress view stops when the channel closes, which only happens
    /// once every sender is gone: a stray clone leaves the installer stuck on
    /// the log screen after the install has finished.
    #[tokio::test]
    async fn the_channel_closes_when_the_sender_is_dropped() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        stream("echo", &["done"], &tx).await.unwrap();
        drop(tx);

        let mut lines = Vec::new();
        while let Some(line) = rx.recv().await {
            lines.push(line);
        }
        assert!(lines.iter().any(|l| l == "done"), "{lines:?}");
    }
}
