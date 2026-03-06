pub mod cli;
pub mod cmd;
pub mod config;
pub mod doctor;
pub mod mcp;
pub mod provision;
pub mod ssh;
pub mod state;
pub mod tart;
pub mod vm;
pub mod worktree;

use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum TachikomaError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("State error: {0}")]
    State(String),

    #[error("Git error: {0}")]
    Git(String),

    #[error("Tart error: {0}")]
    Tart(String),

    #[error("SSH error: {0}")]
    Ssh(String),

    #[error("Provisioning error: {0}")]
    Provision(String),

    #[error("VM error: {0}")]
    Vm(String),

    #[error("MCP error: {0}")]
    Mcp(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, TachikomaError>;

/// Generate VM name: tachikoma-<repo>-<branch-slug>
/// - Lowercase everything
/// - Replace non-alphanumeric with hyphens
/// - Collapse multiple hyphens
/// - Trim leading/trailing hyphens
/// - Truncate to 63 chars
pub fn vm_name(repo: &str, branch: &str) -> String {
    let raw = format!("tachikoma-{}-{}", repo, branch);

    let slugified: String = raw
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();

    let collapsed = slugified
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    let truncated = if collapsed.len() > 63 {
        &collapsed[..63]
    } else {
        &collapsed
    };

    truncated.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_config() {
        let err = TachikomaError::Config("bad toml".into());
        assert_eq!(err.to_string(), "Configuration error: bad toml");
    }

    #[test]
    fn error_display_state() {
        let err = TachikomaError::State("corrupt".into());
        assert_eq!(err.to_string(), "State error: corrupt");
    }

    #[test]
    fn error_display_other() {
        let err = TachikomaError::Other("something went wrong".into());
        assert_eq!(err.to_string(), "something went wrong");
    }

    #[test]
    fn error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err: TachikomaError = io_err.into();
        assert!(matches!(err, TachikomaError::Io(_)));
        assert_eq!(err.to_string(), "file missing");
    }

    #[test]
    fn vm_name_simple_branch() {
        assert_eq!(vm_name("myapp", "main"), "tachikoma-myapp-main");
    }

    #[test]
    fn vm_name_feature_branch() {
        assert_eq!(
            vm_name("myapp", "feature/auth-system"),
            "tachikoma-myapp-feature-auth-system"
        );
    }

    #[test]
    fn vm_name_special_chars() {
        assert_eq!(
            vm_name("myapp", "feat/ABC_123!"),
            "tachikoma-myapp-feat-abc-123"
        );
    }

    #[test]
    fn vm_name_truncation() {
        let long_branch = "a".repeat(100);
        let result = vm_name("myapp", &long_branch);
        assert!(result.len() <= 63);
        assert!(!result.ends_with('-'));
    }

    #[test]
    fn vm_name_empty_branch() {
        let result = vm_name("myapp", "");
        assert_eq!(result, "tachikoma-myapp");
        assert!(!result.ends_with('-'));
    }
}
