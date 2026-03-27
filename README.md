# Tachikoma

Autonomous VM sandboxes per git worktree on Apple Silicon. Named after the think-tanks from Ghost in the Shell.

Tachikoma spawns isolated Linux VMs via [Tart](https://tart.run), one per git branch, with your repo mounted writable, Claude Code installed, and credentials injected automatically. Run `tachikoma` in a repo and you're inside a VM with everything ready. When Claude is done, `tachikoma pr` commits the changes and opens a GitHub PR from the host. A progress spinner shows each step as it happens.

## Installation

Download the latest pre-built binary using the [gh CLI](https://cli.github.com) (required for this private repo):

```bash
# Apple Silicon (M1/M2/M3)
gh release download v0.2.3 --repo ClickHouse/tachikoma --pattern tachikoma-macos-arm64 \
  --output /tmp/tachikoma && sudo mv /tmp/tachikoma /usr/local/bin/tachikoma && sudo chmod +x /usr/local/bin/tachikoma

# Intel Mac
gh release download v0.2.3 --repo ClickHouse/tachikoma --pattern tachikoma-macos-x86_64 \
  --output /tmp/tachikoma && sudo mv /tmp/tachikoma /usr/local/bin/tachikoma && sudo chmod +x /usr/local/bin/tachikoma
```

Or build from source (requires [Rust](https://rustup.rs)):

```bash
git clone https://github.com/ClickHouse/tachikoma && cd tachikoma
cargo build --release && sudo cp target/release/tachikoma /usr/local/bin/
```

## Quick Start

```bash
# Install dependencies
brew install cirruslabs/cli/tart
tart clone ghcr.io/cirruslabs/ubuntu:latest ubuntu

# Build tachikoma
cargo build --release

# Spawn a VM for the current branch
cd your-repo
tachikoma spawn

# You're now SSH'd into an Ubuntu VM with:
#   - Your repo at ~/code (writable virtiofs mount — Claude can edit files)
#   - Git configured (GIT_DIR, GIT_WORK_TREE, safe.directory)
#   - Claude Code installed and authenticated
#   - Hostname set to your branch slug (admin@feature-ui-button)
```

## What Happens on `spawn`

1. Detects current repo and branch (or uses `--branch`)
2. Ensures a git worktree exists for the branch
3. Clones the base image and boots a VM (or reconnects if already running)
4. Mounts the worktree as `code` and `.git` as `dotgit` via virtiofs
5. Mounts safe `~/.claude` subdirectories (rules, agents, plugins, skills, project memory)
6. Generates a dedicated SSH key pair (`~/.ssh/tachikoma`) if needed
7. Injects the SSH key, mounts virtiofs inside the guest, configures git env
8. Sets VM hostname to branch slug (e.g. `admin@feature-ui-button`)
9. Resolves credentials (keychain, env vars, files, commands) and injects them via base64 encoding
10. Installs Claude Code, skips onboarding wizard, and injects cleaned host `settings.json`
11. Syncs `gh` CLI auth from host if `sync_gh_auth = true`
12. Runs any provisioning scripts (warns for repo-level scripts)
13. Drops you into an interactive SSH session

## Commands

```
tachikoma [BRANCH]       Shorthand for `spawn <branch>`
tachikoma spawn [BRANCH] Spawn/reconnect a VM for a branch
tachikoma enter [NAME]   SSH into a running VM
tachikoma exec <CMD>     Run a command in the VM
tachikoma pr [--name N]  Commit Claude's changes and open a GitHub PR
tachikoma cd [NAME]      Print the worktree path (useful for cd $(tachikoma cd))
tachikoma halt [NAME]    Stop a VM
tachikoma suspend [NAME] Suspend a VM (save state to disk)
tachikoma destroy [NAME] Destroy a VM and its state (confirms, skip with --force)
tachikoma list           List all VMs
tachikoma status         Show current VM status
tachikoma prune          Prune VMs unused for 30+ days
tachikoma image          Manage base images (pull/list/delete)
tachikoma doctor         Check prerequisites (tart, git, ssh)
tachikoma config         Show or edit configuration
tachikoma ssh            Manage SSH config entries
tachikoma completions    Generate shell completions (bash/zsh/fish)
tachikoma mcp            Start MCP server (stdio JSON-RPC)
```

All commands support `--json` for machine-readable output and `-v` for verbose logging.

## Configuration

Tachikoma uses layered TOML config: defaults < global < repo < local.

| File | Scope |
|------|-------|
| Built-in defaults | Always applied |
| `~/.config/tachikoma/config.toml` | Global (all repos) |
| `.tachikoma.toml` (repo root) | Per-repo (committed) |
| `.tachikoma.local.toml` (repo root) | Per-repo (gitignored) |

```toml
base_image = "ubuntu"           # Tart image to clone
vm_cpus = 4                     # CPU cores
vm_memory = 8192                # Memory in MB
vm_display = "none"             # "none" for headless
ssh_user = "admin"              # VM SSH user
ssh_port = 22
boot_timeout_secs = 120         # Max wait for VM boot
prune_after_days = 30           # Auto-prune threshold

# Custom worktree location (default: sibling of repo root)
# worktree_dir = "/path/to/worktrees"

# Additional provisioning scripts to run in the VM (see examples/install-tools.sh)
# provision_scripts = ["./examples/install-tools.sh"]

# Sync host's gh CLI auth (~/.config/gh/hosts.yml) into VM (opt-in)
# sync_gh_auth = true

# ~/.claude subdirs to virtiofs-mount and symlink into the VM (default shown)
# share_claude_dirs = ["rules", "agents", "plugins", "skills"]

# Preserve mcpServers from host settings.json and export their env vars into ~.profile
# sync_mcp_servers = true

# Custom credential resolution
# credential_command = "op read op://vault/claude/credential"
# api_key_command = "op read op://vault/anthropic/api-key"

# Credential proxy (enabled by default) — keeps API keys off the VM entirely
# credential_proxy = true
# credential_proxy_port = 19280
# credential_proxy_bind = "192.168.64.1"   # Tart vmnet bridge; use "0.0.0.0" only for testing
# credential_proxy_ttl_secs = 300
```

## Credential Proxy

By default (`credential_proxy = true`), Tachikoma runs a lightweight HTTP proxy on the host that handles all Anthropic API authentication. The VM receives only `ANTHROPIC_BASE_URL` pointing at the proxy — the actual API key or OAuth token never enters the VM.

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

The proxy is auto-started on `tachikoma spawn`, shared across all running VMs, and managed via `tachikoma proxy start/stop/status`. Use `tachikoma proxy --help` for details.

To disable and inject credentials directly into the VM instead, set `credential_proxy = false`.

## Credential Resolution

Credentials are resolved on the host (by the proxy, or injected directly if `credential_proxy = false`), trying sources in order — first match wins:

1. macOS Keychain (`Claude Code-credentials`)
2. `CLAUDE_CODE_OAUTH_TOKEN` env var
3. Configured `credential_command`
4. `~/.claude/.credentials.json` file
5. `ANTHROPIC_API_KEY` env var
6. Configured `api_key_command`
7. Proxy env vars (`CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_USE_VERTEX`, `ANTHROPIC_BASE_URL`)

## VM Filesystem Layout

Inside the VM:

```
/mnt/tachikoma/code/            # Worktree (writable virtiofs — Claude can edit files)
/mnt/tachikoma/dotgit/          # .git directory (writable virtiofs)
/mnt/tachikoma/claude-rules/    # ~/.claude/rules (read-only virtiofs)
/mnt/tachikoma/claude-agents/   # ~/.claude/agents (read-only virtiofs)
/mnt/tachikoma/claude-plugins/  # ~/.claude/plugins (read-only virtiofs)
/mnt/tachikoma/claude-skills/   # ~/.claude/skills (read-only virtiofs)
/mnt/tachikoma/claude-memory/   # Project memory directory (read-only virtiofs)
~/code                           # Symlink to /mnt/tachikoma/code
~/.claude/rules                  # Symlink to /mnt/tachikoma/claude-rules
~/.claude/agents                 # Symlink to /mnt/tachikoma/claude-agents
~/.claude/plugins                # Symlink to /mnt/tachikoma/claude-plugins
~/.claude/skills                 # Symlink to /mnt/tachikoma/claude-skills
~/.claude/settings.json          # Cleaned copy of host settings (writable)
```

Only non-sensitive `~/.claude` subdirectories are mounted. Sensitive data (`history.jsonl`, `projects/`, `debug/`, `file-history/`) is never exposed. The host `settings.json` is stripped of hooks, statusLine, and macOS-specific deny rules before injection.

**Note:** The `.git` directory is mounted writable — Claude has full git access inside the VM (commit, push, branch, worktree). `tachikoma pr` remains available as a convenience from the host.

Git discovers `GIT_DIR` automatically from the `.git` file in the code mount (rewritten during provisioning to point at the VM-local dotgit path). No global `GIT_DIR` or `GIT_WORK_TREE` exports — this keeps `git clone` working for unrelated repos (e.g. Claude Code plugins).

Environment (set in `~/.profile`):
```bash
TACHIKOMA=1
cd /mnt/tachikoma/code               # start in the repo
```

## Typical Workflow

```bash
tachikoma              # spawn VM, SSH in, Claude starts working
tachikoma enter        # re-enter the VM to check progress or direct Claude
tachikoma pr           # when done: commit + push + open GitHub PR
tachikoma destroy      # clean up the VM when the PR is merged
```

`tachikoma pr` auto-generates a commit message from `git diff --stat` and calls `gh pr create --fill`. Requires the [gh CLI](https://cli.github.com) on the host.

## MCP Server

Tachikoma includes an MCP server for integration with Claude and other AI tools:

```bash
tachikoma mcp
```

This starts a stdio JSON-RPC 2.0 server implementing the Model Context Protocol, allowing AI assistants to spawn and manage VMs programmatically.

## Architecture

```
src/
  cli/        Clap arg parsing, output formatting (human/json/verbose)
  cmd/        Command implementations (thin wiring modules)
  config/     Layered TOML config with merge semantics
  doctor/     Prerequisite checks (tart, git, ssh)
  mcp/        JSON-RPC 2.0 types, handler, stdio transport
  provision/  Credential waterfall, SSH key gen, virtiofs mount, Claude install
  ssh/        SSH trait (check, run, interactive), tachikoma key management
  state/      JSON state file with fd-lock advisory locking
  tart/       TartRunner trait, VM types, dir mounts
  vm/         Zero-arg state machine orchestrator, boot detection with backoff
  worktree/   GitWorktree trait, branch detection, worktree management
```

All external dependencies are behind traits (`TartRunner`, `SshClient`, `GitWorktree`, `StateStore`, `ConfigLoader`) for testability with mockall.

## Requirements

- macOS on Apple Silicon
- [Tart](https://tart.run) (`brew install cirruslabs/cli/tart`)
- A Tart-compatible Linux image (e.g. `ghcr.io/cirruslabs/ubuntu:latest`)
- Rust toolchain (for building)

## Development

```bash
cargo test          # run all tests
cargo clippy -- -D warnings
cargo run -- doctor # verify prerequisites
```

## Releasing

Releases are built and published via GitHub Actions for both Apple Silicon and Intel macOS. Only maintainers with write access can trigger a release.

**Steps:**

1. Bump `version` in `Cargo.toml` on a feature branch:
   ```toml
   version = "0.2.2"
   ```
2. Open a PR, get it merged to `main`.
3. Go to **Actions → Release → Run workflow**, enter the version (e.g. `0.2.2`).
4. The workflow validates the version, builds both architectures, creates the git tag `v0.2.3`, and publishes a GitHub Release with the binaries attached.

The released binaries are stripped and statically linked — no Rust toolchain needed to run them.

## License

MIT
