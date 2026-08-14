use thiserror::Error;

#[derive(Error, Debug)]
pub enum NixstallError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Preflight check failed: {0}")]
    Preflight(String),

    #[error("No usable disk found")]
    NoDisk,

    #[error("Disk not found: {0}")]
    DiskNotFound(String),

    #[error("Unknown layout: {0}")]
    LayoutNotFound(String),

    #[error("Unknown profile: {0}")]
    ProfileNotFound(String),

    #[error("Command `{command}` failed with status {status}")]
    Command { command: String, status: i32 },

    #[error("Template error: {0}")]
    Template(String),

    #[error("Cancelled by user")]
    Cancelled,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, NixstallError>;

#[cfg(test)]
mod tests {
    use super::NixstallError;

    #[test]
    fn config_error_display() {
        let err = NixstallError::Config("missing flake".to_string());
        assert_eq!(err.to_string(), "Configuration error: missing flake");
    }

    #[test]
    fn command_error_display() {
        let err = NixstallError::Command {
            command: "disko".to_string(),
            status: 1,
        };
        assert_eq!(err.to_string(), "Command `disko` failed with status 1");
    }
}
