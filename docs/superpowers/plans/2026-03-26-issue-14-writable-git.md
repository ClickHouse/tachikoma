# Issue #14: Worktree Isolation + Writable Git

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix spawn to always create isolated linked worktrees (never reuse main checkout) and make `.git` writable so Claude can use git natively inside the VM.

**Architecture:** Two changes in `src/vm/mod.rs`: (1) `ensure_worktree()` skips main worktree matches and creates a dedicated linked worktree instead, (2) `build_run_opts()` mounts `dotgit` as writable. Update the existing test that asserts read-only.

**Tech Stack:** Rust, mockall, tokio, tempfile

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `src/vm/mod.rs` | Modify lines 104-111, 247 | Skip main worktree reuse; flip dotgit read_only |
| `src/vm/mod.rs` (tests) | Modify lines 636-661; add new test | Update dotgit assertion; add ensure_worktree isolation test |

---

### Task 1: Fix `ensure_worktree` to skip main worktree

**Files:**
- Modify: `src/vm/mod.rs:104-111`
- Test: `src/vm/mod.rs` (test module)

- [ ] **Step 1: Write the failing test**

Add this test to the `mod tests` block in `src/vm/mod.rs`:

```rust
#[tokio::test]
async fn test_ensure_worktree_skips_main_worktree() {
    use crate::worktree::WorktreeInfo;

    let tart = MockTartRunner::new();
    let ssh = MockSshClient::new();
    let state_store = MockStateStore::new();
    let config = test_config();

    let mut git = MockGitWorktree::new();

    // list_worktrees returns the main worktree on "main" branch
    git.expect_list_worktrees().returning(|_| {
        Ok(vec![WorktreeInfo {
            path: PathBuf::from("/tmp/repo"),
            branch: Some("main".to_string()),
            is_main: true,
        }])
    });

    // Expect create_worktree to be called because main worktree should be skipped
    git.expect_create_worktree()
        .withf(|_repo, branch, target| {
            branch == "main" && target.to_string_lossy().contains("myrepo-main")
        })
        .returning(|_, _, target| Ok(target.to_path_buf()));

    let orch = VmOrchestrator::new(&tart, &ssh, &git, &state_store, &config);
    let result = orch
        .ensure_worktree(Path::new("/tmp/repo"), "main", "myrepo")
        .await
        .unwrap();

    // Should NOT be the main worktree path
    assert_ne!(result, PathBuf::from("/tmp/repo"));
    assert!(result.to_string_lossy().contains("myrepo-main"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_ensure_worktree_skips_main_worktree -- --nocapture`

Expected: FAIL — the current code returns `/tmp/repo` (the main worktree) without calling `create_worktree`.

- [ ] **Step 3: Implement the fix**

In `src/vm/mod.rs`, replace lines 103-112:

```rust
        // Check if a worktree already exists for this branch
        for wt in &worktrees {
            if wt.branch.as_deref() == Some(branch) {
                tracing::debug!(
                    "Found existing worktree for branch '{branch}' at {:?}",
                    wt.path
                );
                return Ok(wt.path.clone());
            }
        }
```

With:

```rust
        // Check if a linked worktree already exists for this branch.
        // Skip the main worktree — reusing it breaks isolation (switching
        // branches on the host would change what the VM sees).
        for wt in &worktrees {
            if wt.branch.as_deref() == Some(branch) && !wt.is_main {
                tracing::debug!(
                    "Found existing linked worktree for branch '{branch}' at {:?}",
                    wt.path
                );
                return Ok(wt.path.clone());
            }
        }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test test_ensure_worktree_skips_main_worktree -- --nocapture`

Expected: PASS

- [ ] **Step 5: Also add a test confirming linked worktrees ARE still reused**

Add this test to confirm we didn't break reuse of existing linked worktrees:

```rust
#[tokio::test]
async fn test_ensure_worktree_reuses_existing_linked_worktree() {
    use crate::worktree::WorktreeInfo;

    let tart = MockTartRunner::new();
    let ssh = MockSshClient::new();
    let state_store = MockStateStore::new();
    let config = test_config();

    let mut git = MockGitWorktree::new();

    git.expect_list_worktrees().returning(|_| {
        Ok(vec![
            WorktreeInfo {
                path: PathBuf::from("/tmp/repo"),
                branch: Some("main".to_string()),
                is_main: true,
            },
            WorktreeInfo {
                path: PathBuf::from("/tmp/repo-feature-x"),
                branch: Some("feature-x".to_string()),
                is_main: false,
            },
        ])
    });

    // create_worktree should NOT be called — the linked worktree already exists
    // (mockall will panic if an unexpected call happens)

    let orch = VmOrchestrator::new(&tart, &ssh, &git, &state_store, &config);
    let result = orch
        .ensure_worktree(Path::new("/tmp/repo"), "feature-x", "myrepo")
        .await
        .unwrap();

    assert_eq!(result, PathBuf::from("/tmp/repo-feature-x"));
}
```

- [ ] **Step 6: Run both new tests**

Run: `cargo test test_ensure_worktree -- --nocapture`

Expected: Both `test_ensure_worktree_skips_main_worktree` and `test_ensure_worktree_reuses_existing_linked_worktree` PASS.

- [ ] **Step 7: Commit**

```bash
git add src/vm/mod.rs
git commit -m "fix: skip main worktree in ensure_worktree for isolation (#14)"
```

---

### Task 2: Make dotgit mount writable

**Files:**
- Modify: `src/vm/mod.rs:241-248` (build_run_opts)
- Modify: `src/vm/mod.rs:636-661` (test)

- [ ] **Step 1: Update the existing test to expect writable**

In `src/vm/mod.rs`, find the test `test_build_run_opts_dotgit_mount_is_readonly` (line 637). Replace it:

```rust
#[test]
fn test_build_run_opts_dotgit_mount_is_writable() {
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let worktree = tmp.path().join("worktree");
    let repo_root = tmp.path().join("repo");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::create_dir_all(repo_root.join(".git")).unwrap();

    let config = test_config();
    let tart = MockTartRunner::new();
    let ssh = MockSshClient::new();
    let git = MockGitWorktree::new();
    let state_store = default_state_store();

    let orchestrator = VmOrchestrator::new(&tart, &ssh, &git, &state_store, &config);

    let opts = orchestrator.build_run_opts(&worktree, &repo_root);
    let dotgit = opts
        .dirs
        .iter()
        .find(|d| d.name.as_deref() == Some("dotgit"))
        .unwrap();
    assert!(!dotgit.read_only, "dotgit mount must be writable for git access in VM");
}
```

- [ ] **Step 2: Run the updated test to verify it fails**

Run: `cargo test test_build_run_opts_dotgit_mount_is_writable -- --nocapture`

Expected: FAIL — dotgit is still `read_only: true` in the code.

- [ ] **Step 3: Flip the flag**

In `src/vm/mod.rs`, change line 247:

```rust
                read_only: true,
```

To:

```rust
                read_only: false,
```

Also update the comment on line 241 from `// Mount .git directory read-only` to `// Mount .git directory writable so Claude can use git inside the VM`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test test_build_run_opts_dotgit_mount_is_writable -- --nocapture`

Expected: PASS

- [ ] **Step 5: Run full test suite**

Run: `cargo test`

Expected: All tests pass.

- [ ] **Step 6: Run clippy**

Run: `cargo clippy -- -D warnings`

Expected: No warnings.

- [ ] **Step 7: Commit**

```bash
git add src/vm/mod.rs
git commit -m "feat: make dotgit mount writable for native git access in VM"
```

---

### Task 3: Update CLAUDE.md design constraints

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Update the Key Design Constraints section**

In `CLAUDE.md`, find the constraint:

> **`.git` is read-only in the VM** — Claude can edit source files but cannot run git commands inside the VM. Use `tachikoma pr` on the host to commit and push.

Replace with:

> **`.git` is writable in the VM** — Claude has full git access inside the VM (commit, push, branch, worktree). `tachikoma pr` remains available as a convenience from the host side.

- [ ] **Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: update design constraints for writable git mount"
```
