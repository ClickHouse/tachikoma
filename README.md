# Tachikoma

Autonomous VM sandboxes per git worktree on Apple Silicon. Named after the think-tanks from Ghost in the Shell.

Tachikoma spawns isolated Linux VMs via [Tart](https://tart.run), one per git branch, with your repo mounted read-only, Claude Code installed, and credentials injected automatically. Run `tachikoma` in a repo and you're inside a VM with everything ready.

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
```

## What Happens on `spawn`

1. Detects current repo and branch (or uses `--branch`)
2. Ensures a git worktree exists for the branch
3. Clones the base image and boots a VM (or reconnects if already running)
4. Mounts the worktree as `code` and `.git` as `dotgit` via virtiofs
5. Generates a dedicated SSH key pair (`~/.ssh/tachikoma`) if needed
6. Injects the SSH key, mounts virtiofs inside the guest, configures git env
7. Resolves credentials (keychain, env vars, files, commands) and injects them
8. Installs Claude Code and marks onboarding complete
9. Runs any provisioning scripts
10. Drops you into an interactive SSH session

## Commands

```
tachikoma [BRANCH]       Shorthand for `spawn <branch>`
tachikoma spawn [BRANCH] Spawn/reconnect a VM for a branch
tachikoma enter [NAME]   SSH into a running VM
tachikoma exec <CMD>     Run a command in the VM
tachikoma halt [NAME]    Stop a VM
tachikoma suspend [NAME] Suspend a VM (save state to disk)
tachikoma destroy [NAME] Destroy a VM and its state
tachikoma list           List all VMs
tachikoma status         Show current VM status
tachikoma prune          Prune VMs unused for 30+ days
tachikoma image          Manage base images (pull/list/delete)
tachikoma doctor         Check prerequisites (tart, git, ssh)
tachikoma config         Show or edit configuration
tachikoma ssh            Manage SSH config entries
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
/mnt/tachikoma/code/    # Worktree (read-only virtiofs)
/mnt/tachikoma/dotgit/  # .git directory (read-only virtiofs)
~/code                   # Symlink to /mnt/tachikoma/code
```

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
cargo test          # 104 tests
cargo clippy -- -D warnings
cargo run -- doctor # Verify prerequisites
```

## License

MIT
