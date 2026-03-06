use std::path::PathBuf;

use crate::cli::output::{print_success, OutputMode};
use crate::config::ConfigLoader;
use crate::Result;

pub async fn run(
    edit: bool,
    loader: &dyn ConfigLoader,
    repo_root: Option<PathBuf>,
    mode: OutputMode,
) -> Result<()> {
    if edit {
        let config_path = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("tachikoma")
            .join("config.toml");

        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
        let status = std::process::Command::new(&editor)
            .arg(&config_path)
            .status()
            .map_err(|e| {
                crate::TachikomaError::Config(format!("Failed to open editor: {e}"))
            })?;

        if !status.success() {
            return Err(crate::TachikomaError::Config(
                "Editor exited with error".to_string(),
            ));
        }
        return Ok(());
    }

    let config = loader.load(repo_root).await?;
    let data = serde_json::to_value(&config).map_err(|e| {
        crate::TachikomaError::Config(format!("Failed to serialize config: {e}"))
    })?;

    print_success(mode, "Current configuration", Some(data));
    Ok(())
}
