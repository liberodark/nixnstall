use crate::error::{NixstallError, Result};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, info};

pub async fn capture(program: &str, args: &[&str]) -> Result<String> {
    debug!(program, ?args, "capture");
    let out = Command::new(program).args(args).output().await?;
    if !out.status.success() {
        return Err(NixstallError::Command {
            command: program.to_string(),
            status: out.status.code().unwrap_or(-1),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub async fn stream(program: &str, args: &[&str], log: &UnboundedSender<String>) -> Result<()> {
    debug!(program, ?args, "stream");
    let cmdline = format!("$ {} {}", program, args.join(" "));
    info!("{cmdline}");
    let _ = log.send(cmdline);

    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let out_tx = log.clone();
    let out_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            info!("{line}");
            let _ = out_tx.send(line);
        }
    });

    let err_tx = log.clone();
    let err_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            info!("{line}");
            let _ = err_tx.send(line);
        }
    });

    let status = child.wait().await?;
    let _ = out_task.await;
    let _ = err_task.await;

    if !status.success() {
        return Err(NixstallError::Command {
            command: program.to_string(),
            status: status.code().unwrap_or(-1),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::capture;

    #[tokio::test]
    async fn capture_reads_stdout() {
        let out = capture("echo", &["hello"]).await.unwrap();
        assert_eq!(out.trim(), "hello");
    }

    #[tokio::test]
    async fn capture_reports_failure() {
        assert!(capture("false", &[]).await.is_err());
    }
}

#[cfg(test)]
mod logging {
    use super::stream;

    #[tokio::test]
    async fn streamed_lines_are_logged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.log");
        let file = std::fs::File::create(&path).unwrap();

        let subscriber = tracing_subscriber::fmt()
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        stream("echo", &["marker-line"], &tx).await.unwrap();
        drop(tx);
        while rx.recv().await.is_some() {}

        let logged = std::fs::read_to_string(&path).unwrap();
        assert!(logged.contains("marker-line"), "{logged}");
    }
}
