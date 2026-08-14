use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

mod answers;
mod config;
mod error;
mod probe;
mod runner;
mod steps;
mod tui;

use answers::Answers;
use config::Config;

#[derive(Parser, Debug)]
#[command(name = "nixstall", version, about, long_about = None)]
struct Cli {
    #[arg(
        short,
        long,
        env = "NIXSTALL_CONFIG",
        default_value = "/etc/nixstall/installer.toml"
    )]
    config: PathBuf,

    #[arg(short, long)]
    answers: Option<PathBuf>,

    #[arg(long)]
    dry_run: bool,

    #[arg(long, default_value = "/tmp/nixstall.log")]
    log_file: PathBuf,

    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let filter = match cli.verbose {
        0 => "nixstall=info",
        1 => "nixstall=debug",
        _ => "nixstall=trace",
    };
    let file = std::fs::File::create(&cli.log_file)?;
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(file).with_ansi(false))
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()))
        .init();

    let config = Config::load(&cli.config)?;

    if let Some(path) = cli.answers {
        let answers: Answers = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        probe::ensure_disk(&probe::disks().await?, &answers.device)?;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let printer = tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                println!("{line}");
            }
        });
        let result = steps::Installer {
            config: &config,
            answers: &answers,
            log: tx,
            dry_run: cli.dry_run,
        }
        .run()
        .await;
        let _ = printer.await;
        result?;
        return Ok(());
    }

    let disks = probe::disks().await?;
    let warnings = probe::preflight(&config.check).await?;

    let mut ui = tui::Tui::new()?;
    let outcome = ui.wizard(&config, &disks, &warnings);

    let (mut answers, password) = match outcome {
        Ok(v) => v,
        Err(e) => {
            ui.restore()?;
            return Err(e.into());
        }
    };
    answers.hashed_password = steps::hash_password(&password).await?;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let title = config.installer.title.clone();
    let installer = steps::Installer {
        config: &config,
        answers: &answers,
        log: tx,
        dry_run: cli.dry_run,
    };

    let (result, _) = tokio::join!(installer.run(), ui.progress(&mut rx, &title));
    ui.restore()?;

    match result {
        Ok(()) => {
            println!(
                "Installed. Reboot, then update with:\n  sudo nixos-rebuild switch --flake {}#{}",
                config.installer.target_flake.display(),
                answers.hostname
            );
            Ok(())
        }
        Err(e) => {
            eprintln!("Install failed: {e}");
            eprintln!("Log: {}", cli.log_file.display());
            Err(e.into())
        }
    }
}
