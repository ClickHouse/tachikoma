# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build --release   # build
cargo test              # run all tests
cargo test <name>       # run a single test by name
cargo clippy -- -D warnings  # lint
cargo fmt               # format
make check              # fmt + lint + test
cargo run -- doctor     # verify prerequisites (tart, git, ssh)
```

## Architecture

Tachikoma is a Rust CLI + MCP server (~6,500 lines, 38 files, edition 2021) that spawns isolated Linux VMs per git branch on Apple Silicon via [Tart](https://tart.run). VM names are deterministic: `tachikoma-<repo>-<branch-slug>`.

### Module Layout

```
src/
  lib.rs          TachikomaError, vm_name()
  cli/            Clap arg parsing, output formatting (human/json/verbose)
  cmd/            Thin command wiring (spawn, halt, destroy, pr, list, ...)
  vm/             VmOrchestrator state machine + two-phase boot detection
  provision/      SSH key gen, virtiofs mount, credential waterfall, Claude install
  tart/           TartRunner trait + RealTartRunner (wraps tart CLI)
  worktree/       GitWorktree trait + branch detection + worktree management
  ssh/            SshClient trait (check, run, interactive)
  state/          JSON state file with fd-lock advisory locking + atomic writes
  config/         Layered TOML config (defaults < global < repo < local)
  mcp/            stdio JSON-RPC 2.0 MCP server
  doctor/         Prerequisite checks
```

### Trait-Based DI

All external interactions are behind `#[async_trait]` traits with `#[cfg_attr(test, mockall::automock)]`. Every command and `VmOrchestrator` accepts `&dyn Trait`, enabling full mock-based unit tests:

| Trait | Purpose |
|-------|---------|
| `TartRunner` | All `tart` CLI calls |
| `GitWorktree` | All `git` CLI calls |
| `SshClient` | SSH connectivity |
| `StateStore` | JSON state persistence |
| `ConfigLoader` | TOML config loading |

### Core Spawn Flow

`cmd/spawn::run()` → `ensure_worktree()` → `VmOrchestrator::spawn()` (state machine: Not Found → clone+run; Stopped → run; Suspended → run; Running → reconnect) → `wait_for_boot()` (two-phase: poll `tart ip`, then TCP :22) → `provision_vm()` (only on `SpawnResult::Created`) → `ssh.connect_interactive()` (exec replaces process).

Provisioning steps (in order): inject SSH key → virtiofs mounts → set hostname to branch slug → write env vars to `~/.profile` → resolve + inject credentials → install Claude Code → patch `~/.claude.json` → symlink `~/.claude/{rules,agents,plugins,skills}` → inject stripped `settings.json` → run provisioning scripts.

### Key Design Constraints

- **`.git` is read-only in the VM** — Claude can edit source files but cannot run git commands inside the VM. Use `tachikoma pr` on the host to commit and push.
- **Credentials are base64-encoded** before injection via `tart exec` to avoid shell escaping issues. Env var names are validated against `[A-Z0-9_]+`.
- **`settings.json` is stripped** of `hooks`, `statusLine`, and macOS `~/Library/` deny rules before injection into the VM.
- **State writes are atomic**: serialize to `state.json.tmp`, then `rename()`. Protected by `fd-lock` advisory locking.
- **`tart suspend` is not used** for Linux VMs (breaks them); `suspend` calls `tart stop` instead.

### Config Merge Chain

`defaults.rs` → `~/.config/tachikoma/config.toml` → `<repo>/.tachikoma.toml` → `<repo>/.tachikoma.local.toml`. `PartialConfig` uses `Option<T>` for all fields; later layers win with `other.field.or(self.field)`.

### Credential Waterfall (first match wins)

macOS Keychain → `CLAUDE_CODE_OAUTH_TOKEN` env → `credential_command` config → `~/.claude/.credentials.json` → `ANTHROPIC_API_KEY` env → `api_key_command` config → proxy env vars (Bedrock/Vertex).
