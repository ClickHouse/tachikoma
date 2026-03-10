# Writable Mounts + `tachikoma pr` Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the virtiofs code mount writable so Claude can write files in the VM, and add `tachikoma pr` to commit those changes and open a GitHub PR from the host.

**Architecture:** Drop `:ro` from the `code` DirMount (one-line change in `build_run_opts`). Add a new `src/cmd/pr.rs` that resolves the worktree path from state, builds a commit message from `git diff --stat HEAD`, commits + pushes with `tokio::process::Command`, then shells out to `gh pr create --fill`. Wire it into CLI and `main.rs`.

**Tech Stack:** Rust, Tokio async, Clap derive, mockall for tests, `tokio::process::Command` for git/gh calls.

---

### Task 1: Make the code mount writable

**Files:**
- Modify: `src/vm/mod.rs` (line ~236)

The `build_run_opts` function currently passes `read_only: true` for the `code` DirMount. Change it to `false`.

**Step 1: Write the failing test**

In `src/vm/mod.rs`, inside the `#[cfg(test)] mod tests` block, add a new test after the existing test helpers:

```rust
#[test]
fn test_build_run_opts_code_mount_is_writable() {
    use std::path::PathBuf;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let worktree = tmp.path().join("worktree");
    let repo_root = tmp.path().join("repo");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::create_dir_all(&repo_root).unwrap();

    let config = test_config();
    let tart = MockTartRunner::new();
    let ssh = MockSshClient::new();
    let git = MockGitWorktree::new();
    let state_store = default_state_store();

    let orchestrator = VmOrchestrator::new(
        Arc::new(tart),
        Arc::new(ssh),
        Arc::new(git),
        Arc::new(state_store),
        config,
    );

    let opts = orchestrator.build_run_opts(&worktree, &repo_root);
    let code_mount = opts.dirs.iter().find(|d| d.name.as_deref() == Some("code")).unwrap();
    assert!(!code_mount.read_only, "code mount must be writable");
}

#[test]
fn test_build_run_opts_dotgit_mount_is_readonly() {
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let worktree = tmp.path().join("worktree");
    let repo_root = tmp.path().join("repo");
    std::fs::create_dir_all(&worktree).unwrap();
    // Create .git dir so it gets mounted
    std::fs::create_dir_all(repo_root.join(".git")).unwrap();

    let config = test_config();
    let tart = MockTartRunner::new();
    let ssh = MockSshClient::new();
    let git = MockGitWorktree::new();
    let state_store = default_state_store();

    let orchestrator = VmOrchestrator::new(
        Arc::new(tart),
        Arc::new(ssh),
        Arc::new(git),
        Arc::new(state_store),
        config,
    );

    let opts = orchestrator.build_run_opts(&worktree, &repo_root);
    let dotgit = opts.dirs.iter().find(|d| d.name.as_deref() == Some("dotgit")).unwrap();
    assert!(dotgit.read_only, "dotgit mount must stay read-only");
}
```

**Step 2: Run test to verify it fails**

```bash
cargo test test_build_run_opts_code_mount_is_writable -- --nocapture
```

Expected: FAIL — `assertion failed: !code_mount.read_only`

**Step 3: Make the code mount writable**

In `src/vm/mod.rs`, find `build_run_opts` (around line 234). Change:

```rust
let mut dirs = vec![DirMount {
    name: Some("code".into()),
    host_path: worktree_path.to_path_buf(),
    read_only: true,   // ← change this
}];
```

To:

```rust
let mut dirs = vec![DirMount {
    name: Some("code".into()),
    host_path: worktree_path.to_path_buf(),
    read_only: false,
}];
```

**Step 4: Run tests to verify they pass**

```bash
cargo test test_build_run_opts -- --nocapture
```

Expected: both new tests PASS. Then run the full suite:

```bash
cargo test
```

Expected: all tests pass, no regressions.

**Step 5: Commit**

```bash
git add src/vm/mod.rs
git commit -m "feat: make virtiofs code mount writable for Claude file writes"
```

---

### Task 2: Add `GitWorktree` methods needed by `tachikoma pr`

**Files:**
- Modify: `src/worktree/mod.rs`

The `pr` command needs three git operations not yet on the trait:
1. `git diff --stat HEAD` — check for changes and build commit message
2. `git add -A` — stage all changes
3. `git commit -m <msg>` — commit

Add these to the `GitWorktree` trait and implement them in `RealGitWorktree`.

**Step 1: Write failing tests**

In `src/worktree/mod.rs`, inside the `#[cfg(test)] mod tests` block, add:

```rust
#[tokio::test]
async fn test_diff_stat_empty_on_clean_repo() {
    let (_dir, path) = init_test_repo().await;
    let wt = RealGitWorktree::new();
    let stat = wt.diff_stat(&path).await.unwrap();
    assert!(stat.is_empty(), "clean repo should have empty diff stat");
}

#[tokio::test]
async fn test_diff_stat_shows_changed_files() {
    let (_dir, path) = init_test_repo().await;
    std::fs::write(path.join("hello.txt"), "world").unwrap();
    let wt = RealGitWorktree::new();
    let stat = wt.diff_stat(&path).await.unwrap();
    assert!(stat.contains("hello.txt"), "diff stat should mention changed file");
}

#[tokio::test]
async fn test_add_and_commit() {
    let (_dir, path) = init_test_repo().await;
    std::fs::write(path.join("hello.txt"), "world").unwrap();
    let wt = RealGitWorktree::new();
    wt.add_all(&path).await.unwrap();
    wt.commit(&path, "test: add hello.txt").await.unwrap();

    // Verify the commit exists
    let output = tokio::process::Command::new("git")
        .args(["log", "--oneline", "-1"])
        .current_dir(&path)
        .output()
        .await
        .unwrap();
    let log = String::from_utf8_lossy(&output.stdout);
    assert!(log.contains("test: add hello.txt"));
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test test_diff_stat -- --nocapture
```

Expected: FAIL — method `diff_stat` not found on type `RealGitWorktree`

**Step 3: Add methods to the trait and implement them**

In `src/worktree/mod.rs`, add three methods to the trait:

```rust
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait GitWorktree: Send + Sync {
    async fn current_branch(&self, path: &Path) -> Result<String>;
    async fn find_repo_root(&self, from: &Path) -> Result<PathBuf>;
    async fn list_worktrees(&self, repo: &Path) -> Result<Vec<WorktreeInfo>>;
    async fn create_worktree(&self, repo: &Path, branch: &str, target: &Path) -> Result<PathBuf>;
    // NEW:
    async fn diff_stat(&self, path: &Path) -> Result<String>;
    async fn add_all(&self, path: &Path) -> Result<()>;
    async fn commit(&self, path: &Path, message: &str) -> Result<()>;
}
```

Add implementations in `RealGitWorktree`:

```rust
async fn diff_stat(&self, path: &Path) -> Result<String> {
    let output = tokio::process::Command::new("git")
        .args(["diff", "--stat", "HEAD"])
        .current_dir(path)
        .output()
        .await
        .map_err(|e| crate::TachikomaError::Git(format!("Failed to run git diff: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(crate::TachikomaError::Git(format!(
            "git diff --stat failed: {stderr}"
        )));
    }

    // Also include untracked files in stat (new files not yet staged)
    let untracked_output = tokio::process::Command::new("git")
        .args(["status", "--short"])
        .current_dir(path)
        .output()
        .await
        .map_err(|e| crate::TachikomaError::Git(format!("Failed to run git status: {e}")))?;

    let diff = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let status = String::from_utf8_lossy(&untracked_output.stdout).trim().to_string();

    // If git diff --stat is empty but there are untracked files, return status instead
    if diff.is_empty() && !status.is_empty() {
        return Ok(status);
    }
    Ok(diff)
}

async fn add_all(&self, path: &Path) -> Result<()> {
    let output = tokio::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(path)
        .output()
        .await
        .map_err(|e| crate::TachikomaError::Git(format!("Failed to run git add: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(crate::TachikomaError::Git(format!(
            "git add -A failed: {stderr}"
        )));
    }
    Ok(())
}

async fn commit(&self, path: &Path, message: &str) -> Result<()> {
    let output = tokio::process::Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(path)
        .output()
        .await
        .map_err(|e| crate::TachikomaError::Git(format!("Failed to run git commit: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(crate::TachikomaError::Git(format!(
            "git commit failed: {stderr}"
        )));
    }
    Ok(())
}
```

**Step 4: Run tests to verify they pass**

```bash
cargo test test_diff_stat test_add_and_commit -- --nocapture
```

Expected: all three new tests PASS.

```bash
cargo test
```

Expected: all tests pass.

**Step 5: Commit**

```bash
git add src/worktree/mod.rs
git commit -m "feat: add diff_stat, add_all, commit to GitWorktree trait"
```

---

### Task 3: Implement `src/cmd/pr.rs`

**Files:**
- Create: `src/cmd/pr.rs`
- Modify: `src/cmd/mod.rs` (add `pub mod pr;`)

**Step 1: Write failing tests first**

Create `src/cmd/pr.rs` with tests only (no implementation yet):

```rust
use crate::state::StateStore;
use crate::worktree::GitWorktree;
use crate::Result;
use std::path::Path;

pub async fn run(
    vm_name: &str,
    git: &dyn GitWorktree,
    state_store: &dyn StateStore,
    ssh_user: &str,
) -> Result<String> {
    todo!()
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

        let result = run("missing-vm", &git, &store, "admin").await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("missing-vm"), "error should mention the vm name");
    }

    #[tokio::test]
    async fn test_pr_nothing_to_commit() {
        let mut state = State::new();
        state.add_vm(make_entry("myrepo-main", "main", "/tmp/wt"));

        let mut store = MockStateStore::new();
        store.expect_load().returning(move || Ok(state.clone()));

        let mut git = MockGitWorktree::new();
        // diff_stat returns empty string → nothing to commit
        git.expect_diff_stat()
            .returning(|_| Ok(String::new()));

        let result = run("myrepo-main", &git, &store, "admin").await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Nothing to commit") || msg.contains("nothing"));
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

        // run() will also call push + gh which use std::process::Command (not mocked)
        // so we only verify up to commit in the unit test; push/gh tested manually
        let _ = run("myrepo-feat", &git, &store, "admin").await;
        // If we reached here without panics on mock expectations, add/commit were called
    }
}
```

Add `pub mod pr;` to `src/cmd/mod.rs`.

**Step 2: Run tests to verify they fail**

```bash
cargo test test_pr_ -- --nocapture
```

Expected: FAIL — `todo!()` panics, or compile error until impl exists.

**Step 3: Implement `pr::run()`**

Replace `todo!()` with the real implementation:

```rust
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
    let entry = state.find_vm(vm_name).ok_or_else(|| {
        TachikomaError::Vm(format!("VM '{vm_name}' not found in state"))
    })?;
    let worktree = &entry.worktree_path;
    let branch = &entry.branch;

    // 2. Check for changes
    let stat = git.diff_stat(worktree).await?;
    if stat.trim().is_empty() {
        return Err(TachikomaError::Other(
            "Nothing to commit — no changes detected in worktree".to_string(),
        ));
    }

    // 3. Build commit message: subject + body from diff stat
    let message = format!("chore: Claude changes on {branch}\n\n{stat}");

    // 4. Stage and commit
    git.add_all(worktree).await?;
    git.commit(worktree, &message).await?;

    // 5. Push
    push_branch(worktree, branch)?;

    // 6. Create PR via gh CLI
    let pr_url = create_pr(worktree, branch)?;

    Ok(pr_url)
}

fn push_branch(worktree: &std::path::Path, branch: &str) -> Result<()> {
    let output = std::process::Command::new("git")
        .args(["-C", &worktree.to_string_lossy(), "push", "-u", "origin", branch])
        .output()
        .map_err(|e| TachikomaError::Git(format!("Failed to run git push: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TachikomaError::Git(format!("git push failed: {stderr}")));
    }
    Ok(())
}

fn create_pr(worktree: &std::path::Path, _branch: &str) -> Result<String> {
    let output = std::process::Command::new("gh")
        .args(["pr", "create", "--fill"])
        .current_dir(worktree)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                TachikomaError::Other(
                    "gh CLI not found. Install it to create PRs: https://cli.github.com\n\
                     Or push the branch and open a PR manually.".to_string(),
                )
            } else {
                TachikomaError::Other(format!("Failed to run gh pr create: {e}"))
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        // gh prints the PR URL to stdout even with some warnings
        if !stdout.trim().is_empty() {
            return Ok(stdout.trim().to_string());
        }
        return Err(TachikomaError::Other(format!("gh pr create failed: {stderr}")));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
```

**Step 4: Run tests**

```bash
cargo test test_pr_ -- --nocapture
```

Expected: all three `test_pr_*` tests pass.

```bash
cargo test && cargo clippy -- -D warnings
```

Expected: all pass, no clippy warnings.

**Step 5: Commit**

```bash
git add src/cmd/pr.rs src/cmd/mod.rs
git commit -m "feat: add tachikoma pr command to commit and open GitHub PR"
```

---

### Task 4: Wire `tachikoma pr` into CLI and `main.rs`

**Files:**
- Modify: `src/cli/mod.rs`
- Modify: `src/main.rs`

**Step 1: Add `Pr` variant to the CLI Command enum**

In `src/cli/mod.rs`, add after the `Cd` variant (around line 80):

```rust
/// Commit Claude's changes and open a GitHub PR
Pr {
    /// VM name (defaults to current branch VM)
    name: Option<String>,
},
```

Also add a CLI parse test for the new command in the `#[cfg(test)]` block at the bottom of `src/cli/mod.rs`:

```rust
#[test]
fn test_pr_command_parses() {
    let cli = Cli::try_parse_from(["tachikoma", "pr"]).unwrap();
    assert!(matches!(cli.command, Some(Command::Pr { name: None })));
}

#[test]
fn test_pr_command_with_name() {
    let cli = Cli::try_parse_from(["tachikoma", "pr", "--name", "myrepo-main"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Pr { name: Some(ref n) }) if n == "myrepo-main"
    ));
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test test_pr_command -- --nocapture
```

Expected: compile error — `Pr` variant doesn't exist yet.

**Step 3: Add `Pr` to the enum**

In `src/cli/mod.rs`, add inside `pub enum Command`:

```rust
/// Commit Claude's changes and open a GitHub PR
Pr {
    /// VM name (defaults to current branch VM)
    #[arg(long)]
    name: Option<String>,
},
```

**Step 4: Wire in `main.rs`**

In `src/main.rs`, add a match arm inside the `match cli.command` block. Place it after `Some(Command::Cd { .. })`:

```rust
Some(Command::Pr { name }) => {
    let vm_name = resolve_vm_name(name, &git, &cwd).await?;
    let pr_url = tachikoma::cmd::pr::run(&vm_name, &git, &state_store).await?;
    print_success(mode, &format!("PR created: {pr_url}"), None);
}
```

Also add `Pr` to the `command_needs_tart` exclusion list (pr doesn't need tart):

```rust
fn command_needs_tart(cmd: &Option<Command>) -> bool {
    !matches!(
        cmd,
        Some(Command::Doctor)
            | Some(Command::Completions { .. })
            | Some(Command::Mcp)
            | Some(Command::Config { .. })
            | Some(Command::Pr { .. })   // ← add this
    )
}
```

**Step 5: Run all tests**

```bash
cargo test test_pr_command -- --nocapture
```

Expected: both CLI parse tests PASS.

```bash
cargo test && cargo clippy -- -D warnings
```

Expected: all pass, clippy clean.

**Step 6: Commit**

```bash
git add src/cli/mod.rs src/main.rs
git commit -m "feat: wire tachikoma pr into CLI"
```

---

### Task 5: Final verification

**Step 1: Run the full test suite**

```bash
cargo test
```

Expected: all tests pass (should be 115+ now with the new ones).

**Step 2: Check clippy**

```bash
cargo clippy -- -D warnings
```

Expected: no warnings.

**Step 3: Verify the CLI help shows the new command**

```bash
cargo run -- --help
```

Expected: `pr` appears in the subcommands list with description "Commit Claude's changes and open a GitHub PR".

```bash
cargo run -- pr --help
```

Expected: shows `--name <NAME>` option.

**Step 4: Commit if anything was adjusted**

If no adjustments needed, all prior commits stand. Otherwise:

```bash
git add -A
git commit -m "fix: address final review issues"
```
