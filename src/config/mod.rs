pub mod defaults;
pub mod schema;

pub use defaults::default_config;
pub use schema::{Config, PartialConfig};

use async_trait::async_trait;
use std::path::PathBuf;

use crate::Result;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ConfigLoader: Send + Sync {
    async fn load(&self, repo_root: Option<PathBuf>) -> Result<Config>;
}

#[derive(Default)]
pub struct FileConfigLoader;

impl FileConfigLoader {
    pub fn new() -> Self {
        Self
    }

    fn global_config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("tachikoma")
            .join("config.toml")
    }
}

/// Read a TOML config file, returning `None` if the file doesn't exist.
async fn read_partial(path: &std::path::Path, label: &str) -> Result<Option<PartialConfig>> {
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => {
            let partial = toml::from_str(&contents).map_err(|e| {
                crate::TachikomaError::Config(format!("Failed to parse {label} config: {e}"))
            })?;
            Ok(Some(partial))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(crate::TachikomaError::Config(format!(
            "Failed to read {label} config: {e}"
        ))),
    }
}

#[async_trait]
impl ConfigLoader for FileConfigLoader {
    async fn load(&self, repo_root: Option<PathBuf>) -> Result<Config> {
        let global_path = Self::global_config_path();

        // Read all config files concurrently; merge in layer order afterward.
        let (global, repo, local) = match &repo_root {
            Some(root) => {
                let repo_path = root.join(".tachikoma.toml");
                let local_path = root.join(".tachikoma.local.toml");
                tokio::join!(
                    read_partial(&global_path, "global"),
                    read_partial(&repo_path, "repo"),
                    read_partial(&local_path, "local"),
                )
            }
            None => {
                let (global,) = tokio::join!(read_partial(&global_path, "global"),);
                (global, Ok(None), Ok(None))
            }
        };

        let mut merged = default_config();
        if let Some(p) = global? {
            merged = merged.merge(p);
        }
        if let Some(p) = repo? {
            merged = merged.merge(p);
        }
        if let Some(p) = local? {
            merged = merged.merge(p);
        }

        Config::from_partial(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_load_defaults_only() {
        // With no config files, should return defaults
        let loader = FileConfigLoader::new();
        let config = loader.load(None).await.unwrap();
        assert_eq!(config.base_image, "ubuntu");
        assert_eq!(config.vm_cpus, 4);
        assert_eq!(config.ssh_user, "admin");
    }

    #[tokio::test]
    async fn test_load_repo_config() {
        let dir = TempDir::new().unwrap();
        let config_content = r#"
            base_image = "custom-image"
            vm_cpus = 8
        "#;
        tokio::fs::write(dir.path().join(".tachikoma.toml"), config_content)
            .await
            .unwrap();

        let loader = FileConfigLoader::new();
        let config = loader.load(Some(dir.path().to_path_buf())).await.unwrap();
        assert_eq!(config.base_image, "custom-image");
        assert_eq!(config.vm_cpus, 8);
        // Defaults still apply for unset fields
        assert_eq!(config.ssh_user, "admin");
    }

    #[tokio::test]
    async fn test_local_overrides_repo() {
        let dir = TempDir::new().unwrap();

        tokio::fs::write(
            dir.path().join(".tachikoma.toml"),
            "base_image = \"repo-image\"\nvm_cpus = 4\n",
        )
        .await
        .unwrap();

        tokio::fs::write(dir.path().join(".tachikoma.local.toml"), "vm_cpus = 16\n")
            .await
            .unwrap();

        let loader = FileConfigLoader::new();
        let config = loader.load(Some(dir.path().to_path_buf())).await.unwrap();
        assert_eq!(config.base_image, "repo-image");
        assert_eq!(config.vm_cpus, 16);
    }

    #[tokio::test]
    async fn test_invalid_toml_returns_error() {
        let dir = TempDir::new().unwrap();
        tokio::fs::write(dir.path().join(".tachikoma.toml"), "not valid toml {{{}}")
            .await
            .unwrap();

        let loader = FileConfigLoader::new();
        let result = loader.load(Some(dir.path().to_path_buf())).await;
        assert!(result.is_err());
    }
}
