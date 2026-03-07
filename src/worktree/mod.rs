use async_trait::async_trait;
use std::path::{Path, PathBuf};

use crate::Result;

#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub is_main: bool,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait GitWorktree: Send + Sync {
    async fn current_branch(&self, path: &Path) -> Result<String>;
    async fn find_repo_root(&self, from: &Path) -> Result<PathBuf>;
    async fn list_worktrees(&self, repo: &Path) -> Result<Vec<WorktreeInfo>>;
    async fn create_worktree(&self, repo: &Path, branch: &str, target: &Path) -> Result<PathBuf>;
}

#[derive(Default)]
pub struct RealGitWorktree;

impl RealGitWorktree {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl GitWorktree for RealGitWorktree {
    async fn current_branch(&self, path: &Path) -> Result<String> {
        let output = tokio::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(path)
            .output()
            .await
            .map_err(|e| {
                crate::TachikomaError::Git(format!("Failed to get current branch: {e}"))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::TachikomaError::Git(format!(
                "git rev-parse failed: {stderr}"
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    async fn find_repo_root(&self, from: &Path) -> Result<PathBuf> {
        let output = tokio::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(from)
            .output()
            .await
            .map_err(|e| crate::TachikomaError::Git(format!("Failed to find repo root: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::TachikomaError::Git(format!(
                "Not a git repository: {stderr}"
            )));
        }

        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(PathBuf::from(root))
    }

    async fn list_worktrees(&self, repo: &Path) -> Result<Vec<WorktreeInfo>> {
        let output = tokio::process::Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(repo)
            .output()
            .await
            .map_err(|e| crate::TachikomaError::Git(format!("Failed to list worktrees: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::TachikomaError::Git(format!(
                "git worktree list failed: {stderr}"
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut worktrees = Vec::new();
        let mut current_path: Option<PathBuf> = None;
        let mut current_branch: Option<String> = None;
        let mut is_first = true;

        for line in stdout.lines() {
            if line.starts_with("worktree ") {
                // Save previous worktree if any
                if let Some(path) = current_path.take() {
                    worktrees.push(WorktreeInfo {
                        path,
                        branch: current_branch.take(),
                        is_main: is_first,
                    });
                    is_first = false;
                }
                let path_str = line.strip_prefix("worktree ").unwrap_or("");
                current_path = Some(PathBuf::from(path_str));
            } else if line.starts_with("branch ") {
                let branch_ref = line.strip_prefix("branch ").unwrap_or("");
                // Strip refs/heads/ prefix
                let branch = branch_ref.strip_prefix("refs/heads/").unwrap_or(branch_ref);
                current_branch = Some(branch.to_string());
            }
        }

        // Save last worktree
        if let Some(path) = current_path.take() {
            worktrees.push(WorktreeInfo {
                path,
                branch: current_branch.take(),
                is_main: is_first,
            });
        }

        Ok(worktrees)
    }

    async fn create_worktree(&self, repo: &Path, branch: &str, target: &Path) -> Result<PathBuf> {
        // First try to create worktree for existing branch
        let output = tokio::process::Command::new("git")
            .args(["worktree", "add", &target.to_string_lossy(), branch])
            .current_dir(repo)
            .output()
            .await
            .map_err(|e| crate::TachikomaError::Git(format!("Failed to create worktree: {e}")))?;

        if !output.status.success() {
            // Try creating with -b for new branch
            let output2 = tokio::process::Command::new("git")
                .args(["worktree", "add", "-b", branch, &target.to_string_lossy()])
                .current_dir(repo)
                .output()
                .await
                .map_err(|e| {
                    crate::TachikomaError::Git(format!("Failed to create worktree: {e}"))
                })?;

            if !output2.status.success() {
                let stderr = String::from_utf8_lossy(&output2.stderr);
                return Err(crate::TachikomaError::Git(format!(
                    "git worktree add failed: {stderr}"
                )));
            }
        }

        Ok(target.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn init_test_repo() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();

        tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(&path)
            .output()
            .await
            .unwrap();

        tokio::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&path)
            .output()
            .await
            .unwrap();

        tokio::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&path)
            .output()
            .await
            .unwrap();

        tokio::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(&path)
            .output()
            .await
            .unwrap();

        (dir, path)
    }

    #[tokio::test]
    async fn test_current_branch() {
        let (_dir, path) = init_test_repo().await;
        let wt = RealGitWorktree::new();
        let branch = wt.current_branch(&path).await.unwrap();
        // Default branch could be "main" or "master"
        assert!(!branch.is_empty());
    }

    #[tokio::test]
    async fn test_find_repo_root() {
        let (_dir, path) = init_test_repo().await;
        let wt = RealGitWorktree::new();
        let root = wt.find_repo_root(&path).await.unwrap();
        // Canonicalize to handle /private/tmp vs /tmp on macOS
        let expected = path.canonicalize().unwrap();
        let actual = root.canonicalize().unwrap();
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_list_worktrees() {
        let (_dir, path) = init_test_repo().await;
        let wt = RealGitWorktree::new();
        let trees = wt.list_worktrees(&path).await.unwrap();
        assert!(!trees.is_empty());
        assert!(trees[0].is_main);
    }

    #[tokio::test]
    async fn test_create_worktree() {
        let (_dir, path) = init_test_repo().await;
        let wt = RealGitWorktree::new();
        let target_dir = TempDir::new().unwrap();
        let target = target_dir.path().join("test-worktree");
        let result = wt.create_worktree(&path, "test-branch", &target).await;
        assert!(result.is_ok(), "create_worktree failed: {:?}", result.err());
        assert!(target.exists());

        // Verify the worktree appears in list
        let trees = wt.list_worktrees(&path).await.unwrap();
        assert_eq!(trees.len(), 2);
    }

    #[tokio::test]
    async fn test_find_repo_root_not_a_repo() {
        let dir = TempDir::new().unwrap();
        let wt = RealGitWorktree::new();
        let result = wt.find_repo_root(dir.path()).await;
        assert!(result.is_err());
    }
}
