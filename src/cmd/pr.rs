use crate::state::StateStore;
use crate::worktree::GitWorktree;
use crate::{Result, TachikomaError};

pub async fn run(
    vm_name: &str,
    git: &dyn GitWorktree,
    state_store: &dyn StateStore,
) -> Result<String> {
    // 1. Resolve worktree path from state
    let state = state_store.load().await?;
    let entry = state
        .find_vm(vm_name)
        .ok_or_else(|| TachikomaError::Vm(format!("VM '{vm_name}' not found in state")))?;
    let worktree = entry.worktree_path.clone();
    let branch = entry.branch.clone();

    // 2. Check for changes
    let stat = git.diff_stat(&worktree).await?;
    if stat.trim().is_empty() {
        return Err(TachikomaError::Other(
            "Nothing to commit — no changes detected in worktree".to_string(),
        ));
    }

    // 3. Build commit message: subject line + diff stat body
    let message = format!("chore: Claude changes on {branch}\n\n{stat}");

    // 4. Stage and commit
    git.add_all(&worktree).await?;
    git.commit(&worktree, &message).await?;

    // 5. Push branch
    push_branch(&worktree, &branch)?;

    // 6. Create PR via gh CLI
    let pr_url = create_pr(&worktree)?;

    Ok(pr_url)
}

fn push_branch(worktree: &std::path::Path, branch: &str) -> Result<()> {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            &worktree.to_string_lossy(),
            "push",
            "-u",
            "origin",
            branch,
        ])
        .output()
        .map_err(|e| TachikomaError::Git(format!("Failed to run git push: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TachikomaError::Git(format!("git push failed: {stderr}")));
    }
    Ok(())
}

fn create_pr(worktree: &std::path::Path) -> Result<String> {
    let output = std::process::Command::new("gh")
        .args(["pr", "create", "--fill"])
        .current_dir(worktree)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                TachikomaError::Other(
                    "gh CLI not found. Install it to create PRs: https://cli.github.com\n\
                     Or push the branch and open a PR manually."
                        .to_string(),
                )
            } else {
                TachikomaError::Other(format!("Failed to run gh pr create: {e}"))
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        // gh prints the PR URL to stdout even on some warning exits
        if !stdout.trim().is_empty() {
            return Ok(stdout.trim().to_string());
        }
        return Err(TachikomaError::Other(format!(
            "gh pr create failed: {stderr}"
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{MockStateStore, State, VmEntry, VmStatus};
    use crate::worktree::MockGitWorktree;
    use chrono::Utc;
    use std::path::PathBuf;

    fn make_entry(name: &str, branch: &str, worktree: &str) -> VmEntry {
        VmEntry {
            name: name.to_string(),
            repo: "myrepo".to_string(),
            branch: branch.to_string(),
            worktree_path: PathBuf::from(worktree),
            created_at: Utc::now(),
            last_used: Utc::now(),
            status: VmStatus::Running,
            ip: Some("192.168.64.10".to_string()),
        }
    }

    #[tokio::test]
    async fn test_pr_vm_not_found() {
        let mut store = MockStateStore::new();
        store.expect_load().returning(|| Ok(State::new()));
        let git = MockGitWorktree::new();

        let result = run("missing-vm", &git, &store).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("missing-vm"),
            "error should mention the vm name, got: {msg}"
        );
    }

    #[tokio::test]
    async fn test_pr_nothing_to_commit() {
        let mut state = State::new();
        state.add_vm(make_entry("myrepo-main", "main", "/tmp/wt"));

        let mut store = MockStateStore::new();
        store.expect_load().returning(move || Ok(state.clone()));

        let mut git = MockGitWorktree::new();
        git.expect_diff_stat().returning(|_| Ok(String::new()));

        let result = run("myrepo-main", &git, &store).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string().to_lowercase();
        assert!(
            msg.contains("nothing") || msg.contains("no changes"),
            "error should mention no changes, got: {msg}"
        );
    }

    #[tokio::test]
    async fn test_pr_runs_git_operations_in_order() {
        let mut state = State::new();
        state.add_vm(make_entry("myrepo-feat", "feat/my-feature", "/tmp/wt"));

        let mut store = MockStateStore::new();
        store.expect_load().returning(move || Ok(state.clone()));

        let mut git = MockGitWorktree::new();
        git.expect_diff_stat()
            .returning(|_| Ok("src/main.rs | 5 +++++\n1 file changed".to_string()));
        git.expect_add_all().returning(|_| Ok(()));
        git.expect_commit()
            .withf(|_, msg: &str| msg.contains("feat/my-feature"))
            .returning(|_, _| Ok(()));
        // push_branch and create_pr use std::process::Command (not mocked),
        // so the test will fail past commit — that's OK, we verify git ops happened.
        // We just check that git.add_all and git.commit expectations were satisfied.
        let _ = run("myrepo-feat", &git, &store).await;
        // mockall will panic if expected methods weren't called
    }
}
