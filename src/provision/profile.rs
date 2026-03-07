use std::path::{Path, PathBuf};

use crate::Result;

/// Discover provisioning profile scripts from multiple locations
pub async fn discover_profiles(
    config_dir: &Path,
    repo_root: Option<&Path>,
    extra_scripts: &[String],
) -> Result<Vec<PathBuf>> {
    let mut profiles = Vec::new();

    // 1. Global profiles directory
    let global_dir = config_dir.join("profiles");
    if global_dir.exists() {
        collect_scripts(&global_dir, &mut profiles).await?;
    }

    // 2. Repo-level profiles (these come from the repo and may not be trusted)
    if let Some(root) = repo_root {
        let repo_profiles = root.join(".tachikoma").join("profiles");
        if repo_profiles.exists() {
            let mut repo_scripts = Vec::new();
            collect_scripts(&repo_profiles, &mut repo_scripts).await?;
            if !repo_scripts.is_empty() {
                tracing::warn!(
                    "Running {} repo-level provisioning script(s) from {}",
                    repo_scripts.len(),
                    repo_profiles.display()
                );
                for script in &repo_scripts {
                    tracing::warn!("  {}", script.display());
                }
            }
            profiles.extend(repo_scripts);
        }
    }

    // 3. Extra scripts from config
    for script in extra_scripts {
        let path = PathBuf::from(script);
        if path.exists() {
            profiles.push(path);
        } else {
            tracing::warn!("Provisioning script not found: {script}");
        }
    }

    Ok(profiles)
}

async fn collect_scripts(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = tokio::fs::read_dir(dir).await.map_err(|e| {
        crate::TachikomaError::Provision(format!(
            "Failed to read profiles directory {}: {e}",
            dir.display()
        ))
    })?;

    let mut paths = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|e| {
        crate::TachikomaError::Provision(format!("Failed to read directory entry: {e}"))
    })? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("sh") {
            paths.push(path);
        }
    }

    // Sort for deterministic ordering
    paths.sort();
    out.extend(paths);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_empty_discovery() {
        let dir = TempDir::new().unwrap();
        let profiles = discover_profiles(dir.path(), None, &[]).await.unwrap();
        assert!(profiles.is_empty());
    }

    #[tokio::test]
    async fn test_global_profiles() {
        let dir = TempDir::new().unwrap();
        let profiles_dir = dir.path().join("profiles");
        tokio::fs::create_dir_all(&profiles_dir).await.unwrap();
        tokio::fs::write(profiles_dir.join("setup.sh"), "#!/bin/bash\necho setup")
            .await
            .unwrap();
        tokio::fs::write(profiles_dir.join("readme.txt"), "not a script")
            .await
            .unwrap();

        let profiles = discover_profiles(dir.path(), None, &[]).await.unwrap();
        assert_eq!(profiles.len(), 1);
        assert!(profiles[0].ends_with("setup.sh"));
    }

    #[tokio::test]
    async fn test_repo_profiles() {
        let config_dir = TempDir::new().unwrap();
        let repo_dir = TempDir::new().unwrap();
        let repo_profiles = repo_dir.path().join(".tachikoma").join("profiles");
        tokio::fs::create_dir_all(&repo_profiles).await.unwrap();
        tokio::fs::write(repo_profiles.join("init.sh"), "#!/bin/bash")
            .await
            .unwrap();

        let profiles = discover_profiles(
            config_dir.path(),
            Some(repo_dir.path()),
            &[],
        )
        .await
        .unwrap();
        assert_eq!(profiles.len(), 1);
    }

    #[tokio::test]
    async fn test_scripts_sorted() {
        let dir = TempDir::new().unwrap();
        let profiles_dir = dir.path().join("profiles");
        tokio::fs::create_dir_all(&profiles_dir).await.unwrap();
        tokio::fs::write(profiles_dir.join("02-config.sh"), "").await.unwrap();
        tokio::fs::write(profiles_dir.join("01-setup.sh"), "").await.unwrap();
        tokio::fs::write(profiles_dir.join("03-finalize.sh"), "").await.unwrap();

        let profiles = discover_profiles(dir.path(), None, &[]).await.unwrap();
        assert_eq!(profiles.len(), 3);
        assert!(profiles[0].ends_with("01-setup.sh"));
        assert!(profiles[1].ends_with("02-config.sh"));
        assert!(profiles[2].ends_with("03-finalize.sh"));
    }

    #[tokio::test]
    async fn test_extra_scripts() {
        let dir = TempDir::new().unwrap();
        let script_path = dir.path().join("custom.sh");
        tokio::fs::write(&script_path, "#!/bin/bash").await.unwrap();

        let profiles = discover_profiles(
            dir.path(),
            None,
            &[script_path.to_string_lossy().to_string()],
        )
        .await
        .unwrap();
        assert_eq!(profiles.len(), 1);
    }

    #[tokio::test]
    async fn test_missing_extra_script_skipped() {
        let dir = TempDir::new().unwrap();
        let profiles = discover_profiles(
            dir.path(),
            None,
            &["/nonexistent/script.sh".to_string()],
        )
        .await
        .unwrap();
        assert!(profiles.is_empty());
    }
}
