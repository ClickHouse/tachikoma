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
  cmd/            Thin command wiring (spawn, halt, destroy, pr, list, proxy, ...)
  vm/             VmOrchestrator state machine + two-phase boot detection
  provision/      SSH key gen, virtiofs mount, credential waterfall, Claude install
  proxy/          Credential proxy server (hyper HTTP + TTL cache)
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

`cmd/spawn::run()` → (if `credential_proxy=true`) `ensure_proxy_running()` → `ensure_worktree()` → `VmOrchestrator::spawn()` (state machine: Not Found → clone+run; Stopped → run; Suspended → run; Running → reconnect) → `wait_for_boot()` (two-phase: poll `tart ip`, then TCP :22) → `provision_vm()` (only on `SpawnResult::Created`) → `ssh.connect_interactive()` (exec replaces process).

Provisioning steps (in order): inject SSH key → virtiofs mounts → rewrite `.git` file to VM-local dotgit path → set hostname to branch slug → resolve + inject credentials (or `ANTHROPIC_BASE_URL` when `credential_proxy=true`) → install Claude Code → patch `~/.claude.json` → symlink configured `~/.claude` subdirs → inject stripped `settings.json` + MCP env vars → run provisioning scripts.

### Credential Proxy

When `credential_proxy = true`, a lightweight HTTP reverse-proxy runs on the host:

```
VM (Linux)                          HOST (macOS)
┌──────────────┐                    ┌─────────────────────────┐
│ Claude Code   │──── HTTP ────────▶│ tachikoma proxy :19280  │
│ ANTHROPIC_    │  (no auth header) │  TTL cache + waterfall  │──▶ api.anthropic.com
│ BASE_URL=     │◀── SSE response ──│  (Keychain/env/command) │    (with auth header)
│ http://192.   │                   └─────────────────────────┘
│ 168.64.1:     │
│ 19280         │
└──────────────┘
```

- **Zero credentials in VM**: only `ANTHROPIC_BASE_URL` is injected; API keys never enter the VM.
- **Auto-started**: `tachikoma spawn` TCP-probes the bind address and starts the proxy as a detached daemon (`setsid`) if not already running.
- **Shared across VMs**: one proxy serves all running VMs; lifecycle is independent of individual VMs.
- **PID file**: `~/.config/tachikoma/proxy.pid` — used by `tachikoma proxy stop`.
- **TTL cache**: credentials resolved once, refreshed after `credential_proxy_ttl_secs` (default 300 s).
- **`GET /health`**: returns `{"status":"ok","proxy":"tachikoma"}` for diagnostics.

### Key Design Constraints

- **`.git` is writable in the VM** — Claude has full git access inside the VM (commit, push, branch, worktree). `tachikoma pr` remains available as a convenience from the host side.
- **Credentials are base64-encoded** before injection via `tart exec` to avoid shell escaping issues. Credential values are single-quoted with POSIX escaping in `~/.profile`. Proxy env var names are validated against `[A-Z0-9_]+`; MCP env var names allow lowercase (`[a-zA-Z_][a-zA-Z0-9_]*`).
- **`settings.json` is stripped** of `hooks`, `statusLine`, and macOS `~/Library/` deny rules before injection into the VM. `mcpServers` is preserved (or stripped when `sync_mcp_servers = false`).
- **`share_claude_dirs`** entries are validated to `[a-zA-Z0-9_-]` only — no slashes or `..` — to prevent path traversal from repo-level config.
- **State writes are atomic**: serialize to `state.json.tmp`, then `rename()`. Protected by `fd-lock` advisory locking.
- **`tart suspend` is not used** for Linux VMs (breaks them); `suspend` calls `tart stop` instead.

### Config Merge Chain

`defaults.rs` → `~/.config/tachikoma/config.toml` → `<repo>/.tachikoma.toml` → `<repo>/.tachikoma.local.toml`. `PartialConfig` uses `Option<T>` for all fields; later layers win with `other.field.or(self.field)`.

### Config Reference

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `base_image` | string | `"ubuntu"` | Tart base image to clone |
| `vm_cpus` | u32 | `4` | vCPU count |
| `vm_memory` | u32 | `8192` | RAM in MB |
| `vm_display` | string | `"none"` | Display mode (`"none"` = headless) |
| `ssh_user` | string | `"admin"` | SSH username inside VM |
| `ssh_port` | u16 | `22` | SSH port |
| `worktree_dir` | path | parent of repo | Directory for linked worktrees |
| `provision_scripts` | string[] | `[]` | Extra scripts to run after provisioning |
| `claude_flags` | string[] | `[]` | Extra flags passed to `claude` |
| `boot_timeout_secs` | u64 | `120` | Max seconds to wait for boot |
| `prune_after_days` | u64 | `30` | Auto-prune VMs older than N days |
| `credential_command` | string | — | Shell command whose stdout is an OAuth token |
| `api_key_command` | string | — | Shell command whose stdout is an API key |
| `sync_gh_auth` | bool | `false` | Sync host `gh` CLI auth into VM |
| `share_claude_dirs` | string[] | `["rules","agents","plugins","skills"]` | `~/.claude` subdirs to virtiofs-mount and symlink into VM. Entries must match `[a-zA-Z0-9_-]`. |
| `sync_mcp_servers` | bool | `true` | When true: preserve `mcpServers` in injected `settings.json` and export each server's `env` vars into `~/.profile`. When false: strip `mcpServers` entirely. |
| `credential_proxy` | bool | `true` | Enable the built-in credential proxy. VM gets `ANTHROPIC_BASE_URL` instead of raw credentials. |
| `credential_proxy_port` | u16 | `19280` | Port the credential proxy listens on. |
| `credential_proxy_bind` | string | `"192.168.64.1"` | Address to bind (Tart vmnet bridge). Use `"0.0.0.0"` only for testing. |
| `credential_proxy_ttl_secs` | u64 | `300` | How long to cache resolved credentials before re-running the waterfall. |

### Credential Waterfall (first match wins)

macOS Keychain → `CLAUDE_CODE_OAUTH_TOKEN` env → `credential_command` config → `~/.claude/.credentials.json` → `ANTHROPIC_API_KEY` env → `api_key_command` config → proxy env vars (Bedrock/Vertex).
