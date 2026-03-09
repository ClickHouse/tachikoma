# Tachikoma

Autonomous VM sandboxes per git worktree on Apple Silicon. Named after the think-tanks from Ghost in the Shell.

Tachikoma spawns isolated Linux VMs via [Tart](https://tart.run), one per git branch, with your repo mounted read-only, Claude Code installed, and credentials injected automatically. Run `tachikoma` in a repo and you're inside a VM with everything ready. A progress spinner shows each step as it happens.

## Installation

Download the latest pre-built binary from [Releases](https://github.com/ClickHouse/tachikoma/releases/latest):

```bash
# Apple Silicon (M1/M2/M3)
curl -fsSL https://github.com/ClickHouse/tachikoma/releases/latest/download/tachikoma-macos-arm64 \
  -o /usr/local/bin/tachikoma && chmod +x /usr/local/bin/tachikoma

# Intel Mac
curl -fsSL https://github.com/ClickHouse/tachikoma/releases/latest/download/tachikoma-macos-x86_64 \
  -o /usr/local/bin/tachikoma && chmod +x /usr/local/bin/tachikoma
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
#   - Your repo at ~/code (read-only virtiofs mount)
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

# Additional provisioning scripts to run in the VM
# provision_scripts = ["setup.sh"]

# Sync host's gh CLI auth (~/.config/gh/hosts.yml) into VM (opt-in)
# sync_gh_auth = true

# Custom credential resolution
# credential_command = "op read op://vault/claude/credential"
# api_key_command = "op read op://vault/anthropic/api-key"
```

## Credential Resolution

Tachikoma automatically finds and injects Claude credentials into the VM, trying sources in order:

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
/mnt/tachikoma/code/            # Worktree (read-only virtiofs)
/mnt/tachikoma/dotgit/          # .git directory (read-only virtiofs)
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

Environment (set in `~/.profile`):
```bash
GIT_DIR=/mnt/tachikoma/dotgit        # or .../worktrees/<branch> for linked worktrees
GIT_WORK_TREE=/mnt/tachikoma/code
TACHIKOMA=1
```

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

1. Bump `version` in `Cargo.toml` on `main`:
   ```toml
   version = "0.2.0"
   ```
2. Open a PR, get it merged.
3. Go to **Actions → Release → Run workflow**, enter the version (e.g. `0.2.0`).
4. The workflow validates the version, builds both architectures, creates the git tag `v0.2.0`, and publishes a GitHub Release with the binaries attached.

The released binaries are stripped and statically linked — no Rust toolchain needed to run them.

## License

MIT
