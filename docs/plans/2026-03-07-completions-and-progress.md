# Shell Completions + Dynamic Progress Spinner

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add shell completions for bash/zsh/fish and a dynamic progress spinner during spawn/provisioning.

**Architecture:** Shell completions via `clap_complete` hidden subcommand. Progress via `indicatif` spinner driven by a callback (`Box<dyn Fn(&str) + Send + Sync>`) threaded through orchestrator and provisioning. Caller in `main.rs` owns the spinner; core logic just calls the callback. JSON mode suppresses the spinner.

**Tech Stack:** `clap_complete` 4.x, `indicatif` 0.17.x

---

### Task 1: Add `clap_complete` dependency

**Files:**
- Modify: `Cargo.toml`

**Step 1: Add dependency**

```toml
clap_complete = "4"
```

Add under `[dependencies]` next to clap.

**Step 2: Verify it compiles**

Run: `cargo check`
Expected: compiles with no errors

---

### Task 2: Add `completions` subcommand

**Files:**
- Modify: `src/cli/mod.rs`

**Step 1: Add the subcommand variant**

Add to `Command` enum:

```rust
/// Generate shell completions
#[command(hide = true)]
Completions {
    /// Shell to generate completions for
    shell: clap_complete::Shell,
},
```

Add `use clap_complete::Shell;` is not needed since we use the full path.

**Step 2: Run existing tests**

Run: `cargo test -p tachikoma cli::tests`
Expected: all pass (existing tests don't break)

**Step 3: Add test for completions parsing**

```rust
#[test]
fn test_completions_command() {
    let cli = Cli::try_parse_from(["tachikoma", "completions", "bash"]).unwrap();
    assert!(matches!(cli.command, Some(Command::Completions { .. })));
}
```

**Step 4: Run test**

Run: `cargo test -p tachikoma cli::tests::test_completions_command`
Expected: PASS

---

### Task 3: Wire completions in main.rs

**Files:**
- Modify: `src/main.rs`

**Step 1: Add handler**

In the `match cli.command` block, add before `Some(Command::Mcp)`:

```rust
Some(Command::Completions { shell }) => {
    clap_complete::generate(
        shell,
        &mut Cli::command(),
        "tachikoma",
        &mut std::io::stdout(),
    );
}
```

Add `use clap::CommandFactory;` at the top of main.rs (needed for `Cli::command()`).

**Step 2: Verify it works**

Run: `cargo run -- completions bash | head -5`
Expected: bash completion script output starting with `_tachikoma()`

Run: `cargo run -- completions zsh | head -5`
Expected: zsh completion script output

Run: `cargo run -- completions fish | head -5`
Expected: fish completion script output

**Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock src/cli/mod.rs src/main.rs
git commit -m "feat: add shell completions via clap_complete (bash/zsh/fish)"
```

---

### Task 4: Add `indicatif` dependency

**Files:**
- Modify: `Cargo.toml`

**Step 1: Add dependency**

```toml
indicatif = "0.17"
```

**Step 2: Verify**

Run: `cargo check`
Expected: compiles

---

### Task 5: Define the progress callback type and add to spawn flow

**Files:**
- Modify: `src/cmd/spawn.rs`

**Step 1: Add `on_status` callback parameter to `cmd::spawn::run`**

```rust
pub async fn run(
    branch: Option<&str>,
    cwd: &Path,
    tart: &dyn TartRunner,
    ssh: &dyn SshClient,
    git: &dyn GitWorktree,
    state_store: &dyn StateStore,
    config: &Config,
    interactive: bool,
    on_status: &dyn Fn(&str),
) -> Result<SpawnResult> {
```

Call `on_status` at each major step:

```rust
on_status("Resolving branch...");
let branch = orch.resolve_branch(branch, cwd).await?;
let (repo_name, repo_root) = orch.resolve_repo(cwd).await?;

on_status("Preparing worktree...");
let worktree_path = orch.ensure_worktree(&repo_root, &branch, &repo_name).await?;

on_status("Spawning VM...");
let result = orch.spawn(&branch, &repo_name, &worktree_path, &repo_root).await?;

if matches!(result, SpawnResult::Created { .. }) {
    on_status("Provisioning VM...");
    provision_vm(tart, ssh, result.ip(), result.name(), &branch, &repo_root, config).await?;
}
```

**Step 2: Fix the test to pass a no-op callback**

In `tests` module, update the `run()` call:

```rust
let result = run(
    None,
    Path::new("/tmp/myrepo"),
    &tart,
    &ssh,
    &git,
    &state_store,
    &config,
    false,
    &|_| {},
)
```

**Step 3: Run tests**

Run: `cargo test -p tachikoma cmd::spawn::tests`
Expected: PASS

---

### Task 6: Add status callbacks to VmOrchestrator::spawn

**Files:**
- Modify: `src/vm/mod.rs`

**Step 1: Add `on_status` parameter to `spawn()`**

```rust
pub async fn spawn(
    &self,
    branch: &str,
    repo_name: &str,
    worktree_path: &Path,
    repo_root: &Path,
    on_status: &dyn Fn(&str),
) -> Result<SpawnResult> {
```

Add calls within the match arms:

```rust
Some(TartVmState::Running) => {
    on_status("Connecting to running VM...");
    // ... existing code
}
Some(TartVmState::Suspended) => {
    on_status("Resuming suspended VM...");
    // ... existing code
    on_status("Waiting for boot...");
    let ip = self.wait_boot(vm_name).await?;
    // ...
}
Some(TartVmState::Stopped) => {
    on_status("Starting stopped VM...");
    // ... existing code
    on_status("Waiting for boot...");
    let ip = self.wait_boot(vm_name).await?;
    // ...
}
Some(TartVmState::Unknown) | None => {
    on_status(&format!("Cloning base image '{}'...", self.config.base_image));
    self.tart.clone_vm(&self.config.base_image, &vm_name).await?;
    on_status("Starting VM...");
    // ... existing code
    on_status("Waiting for boot...");
    let ip = self.wait_boot(vm_name).await?;
    // ...
}
```

**Step 2: Update `cmd/spawn.rs` to pass through**

```rust
let result = orch
    .spawn(&branch, &repo_name, &worktree_path, &repo_root, on_status)
    .await?;
```

**Step 3: Fix all tests in `vm/mod.rs`**

Every test calling `.spawn()` needs the extra `&|_| {}` parameter:

```rust
let result = orch
    .spawn("main", "myrepo", Path::new("/tmp/wt"), Path::new("/tmp/repo"), &|_| {})
    .await
    .unwrap();
```

**Step 4: Run tests**

Run: `cargo test -p tachikoma vm::tests`
Expected: all pass

---

### Task 7: Add status callbacks to provision_vm

**Files:**
- Modify: `src/provision/mod.rs`

**Step 1: Add `on_status` parameter**

```rust
pub async fn provision_vm(
    tart: &dyn TartRunner,
    ssh: &dyn SshClient,
    ip: IpAddr,
    vm_name: &str,
    branch: &str,
    repo_root: &std::path::Path,
    config: &Config,
    on_status: &dyn Fn(&str),
) -> Result<()> {
```

Add calls before each major step:

```rust
on_status("Injecting SSH keys...");
inject_ssh_key(tart, vm_name, &config.ssh_user).await?;

on_status("Mounting shared directories...");
mount_and_configure_git(tart, vm_name, branch).await?;

on_status("Configuring environment...");
// ... TACHIKOMA=1, git config

on_status("Injecting credentials...");
// ... credential injection

on_status("Installing Claude Code...");
install_claude(tart, vm_name).await?;

on_status("Linking host configuration...");
link_host_claude_config(tart, vm_name, repo_root).await;

on_status("Running provisioning scripts...");
// ... profile scripts

on_status("Verifying SSH connectivity...");
// ... ssh verify
```

**Step 2: Update callers**

In `cmd/spawn.rs`:
```rust
provision_vm(tart, ssh, result.ip(), result.name(), &branch, &repo_root, config, on_status).await?;
```

**Step 3: Fix provision tests**

Update all `provision_vm()` calls in tests to pass `&|_| {}`:

```rust
let result = provision_vm(&tart, &ssh, ip, "test-vm", "main", Path::new("/tmp/repo"), &config, &|_| {}).await;
```

**Step 4: Run tests**

Run: `cargo test -p tachikoma provision::tests`
Expected: all pass

---

### Task 8: Wire indicatif spinner in main.rs

**Files:**
- Modify: `src/main.rs`

**Step 1: Create spinner helper**

At top of file or in a helper block:

```rust
use indicatif::{ProgressBar, ProgressStyle};

fn make_spinner(mode: OutputMode) -> ProgressBar {
    if mode == OutputMode::Json {
        return ProgressBar::hidden();
    }
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap()
    );
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb
}
```

**Step 2: Use spinner in spawn paths**

In the `None =>` arm (zero-arg spawn) and `Some(Command::Spawn { .. })` arm:

```rust
None => {
    let branch = cli.branch.as_deref();
    let spinner = make_spinner(mode);
    let on_status = |msg: &str| { spinner.set_message(msg.to_string()); };
    let result = tachikoma::cmd::spawn::run(
        branch, &cwd, &tart, &ssh, &git, &state_store, &config,
        true, &on_status,
    ).await;
    spinner.finish_and_clear();
    let result = result?;
    // ... print_success
}
```

Same pattern for `Some(Command::Spawn { branch })`.

**Step 3: Verify manually**

Run: `cargo run -- spawn` (in a git repo)
Expected: spinner shows status messages during spawn, clears when done

Run: `cargo run -- --json spawn`
Expected: no spinner, JSON output only

**Step 4: Run full test suite**

Run: `cargo test`
Expected: all tests pass

**Step 5: Clippy**

Run: `cargo clippy -- -D warnings`
Expected: clean

**Step 6: Commit**

```bash
git add -A
git commit -m "feat: dynamic progress spinner during spawn/provisioning"
```

---

### Task 9: Final verification

**Step 1: Full test + clippy**

Run: `cargo test && cargo clippy -- -D warnings`
Expected: all pass, clean

**Step 2: Manual E2E test**

```bash
# Test completions
cargo run -- completions bash | head -3
cargo run -- completions zsh | head -3

# Test spinner (destroy + respawn)
cargo run -- destroy tachikoma-tachikoma-main --force
cargo run -- spawn  # should show spinner through all steps

# Test JSON mode suppresses spinner
cargo run -- --json spawn
```

**Step 3: Commit all remaining changes**

```bash
git add -A
git commit -m "chore: final verification of completions + progress features"
```
