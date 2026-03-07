# Architecture

## Overview

Tachikoma is a Rust CLI + MCP server that spawns isolated Linux VMs per git worktree on Apple Silicon, using [Tart](https://tart.run) as the virtualization backend. Each VM is named deterministically from `tachikoma-<repo>-<branch>`, provisioned with SSH keys and Claude Code credentials, and mounted with the host worktree via virtiofs.

~6,200 lines of Rust across 36 source files, 112 tests, edition 2021.

---

## Module Dependency Graph

```
                                 main.rs
                                    |
                    +---------------+----------------+
                    |                                |
                 cli/mod.rs                      lib.rs
                 cli/output.rs           (TachikomaError, vm_name)
                    |                                |
                    v                                v
               cmd/  (thin wiring)          +-------+-------+-------+
               |  |  |  |  |  |            |       |       |       |
               v  v  v  v  v  v            v       v       v       v
            spawn halt destroy ...      config/  state/  tart/  worktree/
               |                           |       |       |       |
               +---------------------------+       |       |       |
               |                                   |       |       |
               v                                   |       |       |
            vm/mod.rs  (VmOrchestrator)            |       |       |
            vm/boot.rs (wait_for_boot)             |       |       |
               |                                   |       |       |
               v                                   |       |       |
          provision/mod.rs                         |       |       |
          provision/credentials.rs                 |       |       |
          provision/profile.rs                     |       |       |
               |                                   |       |       |
               v                                   v       v       v
            ssh/mod.rs  <-------- uses traits ---  (all via dyn trait refs)
               |
               v
          mcp/mod.rs  (stdio JSON-RPC server)
          mcp/types.rs
          mcp/handler.rs
               |
               v
          doctor/mod.rs
```

### Trait Boundaries (DI Seams)

All external interactions are behind `#[async_trait]` traits with `#[cfg_attr(test, mockall::automock)]`:

| Trait | File | Implementor | Purpose |
|-------|------|-------------|---------|
| `TartRunner` | `tart/mod.rs` | `RealTartRunner` | All tart CLI interactions |
| `GitWorktree` | `worktree/mod.rs` | `RealGitWorktree` | Git CLI interactions |
| `SshClient` | `ssh/mod.rs` | `RealSshClient` | SSH connectivity |
| `StateStore` | `state/mod.rs` | `FileStateStore` | JSON state persistence |
| `ConfigLoader` | `config/mod.rs` | `FileConfigLoader` | TOML config loading |

Every command module and the `VmOrchestrator` accept `&dyn Trait` references, enabling full mock-based testing.

---

## Spawn Flow (Core Path)

```
User: `tachikoma` or `tachikoma spawn <branch>`
  |
  v
cmd/spawn::run()
  |
  +-- resolve_branch()
  |     \-- git rev-parse --abbrev-ref HEAD  (if no explicit branch)
  |
  +-- resolve_repo()
  |     \-- git rev-parse --show-toplevel
  |
  +-- ensure_worktree()
  |     +-- git worktree list --porcelain
  |     \-- git worktree add <target> <branch>  (if not found)
  |
  +-- spawn()  <=== STATE MACHINE
  |     +-- tart list --format json      (query VM state)
  |     +-- [tart clone <base> <name>]   (if VM doesn't exist)
  |     +-- [tart run <name> --dir ...]  (if not running)
  |     +-- wait_for_boot()
  |     |     +-- poll: tart ip <name>   (Phase 1: get IP)
  |     |     \-- poll: TCP connect :22  (Phase 2: SSH port open)
  |     \-- state_store.save()
  |
  +-- provision_vm()                     (only on SpawnResult::Created)
  |     +-- inject_ssh_key()             (generate + inject ~/.ssh/tachikoma)
  |     +-- mount_and_configure_git()    (virtiofs mount + GIT_DIR/GIT_WORK_TREE)
  |     +-- set_hostname(branch_slug)    (e.g. admin@feature-ui-button)
  |     +-- set TACHIKOMA=1, git config
  |     +-- resolve + inject credentials (base64-encoded for shell safety)
  |     +-- install_claude()             (curl install.sh | bash)
  |     +-- patch ~/.claude.json         (hasCompletedOnboarding: true)
  |     +-- link_host_claude_config()    (symlink rules/agents/plugins/skills/memory)
  |     +-- inject settings.json         (stripped of hooks/statusLine/macOS deny rules)
  |     +-- run provisioning scripts     (warns for repo-level scripts)
  |     \-- verify SSH works
  |
  \-- ssh.connect_interactive()          (exec() replaces process)
```

---

## VM Lifecycle State Machine

```
                          +-------------+
                          |  Not Found  |
                          | (no tart VM)|
                          +------+------+
                                 |
                        tart clone + tart run
                                 |
                                 v
    +--------+  tart run  +------+------+  tart stop   +----------+
    |Suspended| --------->|   Running   |<------------>|  Stopped  |
    +----+----+           +------+------+              +-----+-----+
         ^                       |                           |
         |                  tart stop                  tart run
         |                       |                           |
         |                       v                           |
         +<-- (disabled)  +------+------+   boot ok   -------+
                          | wait_boot   |------>------/
                          |  (polling)  |
                          +------+------+
                                 |
                            boot timeout
                                 |
                                 v
                           TachikomaError::Vm
```

Note: `suspend` currently calls `tart stop` because `tart suspend` breaks Linux VMs.

### State Transitions

| Current | Command | Action | New State |
|---------|---------|--------|-----------|
| Not Found | `spawn` | clone + run + boot + provision | Running |
| Running | `spawn` | get IP, verify SSH | Running (reconnect) |
| Stopped | `spawn` | run + boot | Running |
| Suspended | `spawn` | run + boot | Running |
| Running | `halt` | stop | Stopped |
| Any | `destroy` | confirm + stop + delete | Removed (--force skips prompt) |
| Stale | `prune` | stop + delete | Removed |

### Boot Detection (Two-Phase with Backoff)

```
Phase 1: IP Acquisition              Phase 2: SSH Port Check
+-------------------+                +-------------------+
| poll tart ip      |  IP obtained   | TCP connect :22   |
| backoff: 500ms    | -------------->| backoff: 500ms    |
| factor: 2x        |                | factor: 2x        |
| max: 5s interval  |                | max: 5s interval  |
| timeout: 120s     |                | timeout: remaining |
+-------------------+                +-------------------+
```

---

## Config Merge Chain

```
+------------------+     +---------------------+     +------------------+     +-------------------------+
|    Hardcoded     |     |    Global Config     |     |   Repo Config    |     |     Local Config        |
|    Defaults      | --> | ~/.config/tachikoma/ | --> | <repo>/.tachikoma| --> | <repo>/.tachikoma.local |
| (defaults.rs)    |     |    config.toml       |     |    .toml         |     |    .toml                |
+------------------+     +---------------------+     +------------------+     +-------------------------+
       Layer 0                  Layer 1                   Layer 2                     Layer 3
```

Merge: `PartialConfig` uses `Option<T>` for all fields. `other.field.or(self.field)` — later layers win, `None` preserves earlier values. `Config::from_partial()` applies defaults for remaining `None`.

| Field | Default |
|-------|---------|
| `base_image` | `"ubuntu"` |
| `vm_cpus` | `4` |
| `vm_memory` | `8192` MB |
| `vm_display` | `"none"` |
| `ssh_user` | `"admin"` |
| `ssh_port` | `22` |
| `boot_timeout_secs` | `120` |
| `prune_after_days` | `30` |
| `sync_gh_auth` | `false` |

---

## Credential Waterfall

First match wins. Resolution in `provision/credentials.rs`:

```
Priority  Source                              Injection in VM
--------  ------                              ---------------
1         macOS Keychain "Claude Code"        ANTHROPIC_API_KEY in ~/.profile
2         CLAUDE_CODE_OAUTH_TOKEN env var     CLAUDE_CODE_OAUTH_TOKEN in ~/.profile
3         credential_command (config)         CLAUDE_CODE_OAUTH_TOKEN in ~/.profile
4         ~/.claude/.credentials.json         ~/.claude/.credentials.json
5         ANTHROPIC_API_KEY env var           ANTHROPIC_API_KEY in ~/.profile
6         api_key_command (config)            ANTHROPIC_API_KEY in ~/.profile
7         Proxy env vars (Bedrock/Vertex)     All related vars in ~/.profile
8         None                                (warning logged)
```

All credential values are base64-encoded before injection (`echo <b64> | base64 -d >> ~/.profile`) to avoid shell escaping and injection issues. Environment variable names are validated against `[A-Z0-9_]+`.

**Supplementary:** `"Claude Code-credentials"` keychain entry (MCP OAuth tokens) is always injected to `~/.claude/.credentials.json` when available, independent of the primary credential.

---

## Key Types

### Error Hierarchy

```
TachikomaError (thiserror + miette::Diagnostic)
  +-- Config(String)
  +-- State(String)
  +-- Git(String)
  +-- Tart(String)
  +-- Ssh(String)
  +-- Provision(String)
  +-- Vm(String)
  +-- Mcp(String)
  +-- Io(#[from] std::io::Error)
  \-- Other(String)
```

### Core Structs

```
VmOrchestrator<'a>             SpawnResult
  tart: &dyn TartRunner          Reconnected { name, ip }
  ssh: &dyn SshClient            Resumed { name, ip }
  git: &dyn GitWorktree          Started { name, ip }
  state_store: &dyn StateStore   Created { name, ip }
  config: &Config

VmEntry                        CredentialSource
  name, repo, branch             Keychain(String)
  worktree_path                  EnvVar(String)
  created_at, last_used          Command(String)
  status: VmStatus               File(String)
  ip: Option<String>             ApiKey(String)
                                 ApiKeyCommand(String)
DirMount                         ProxyEnv { provider, vars }
  name: Option<String>           None
  host_path: PathBuf
  read_only: bool
```

---

## External Process Interactions

### Host-Side

| Binary | Key Invocations | Called By |
|--------|----------------|-----------|
| `tart` | `list`, `clone`, `run` (detached via setsid, return value checked), `stop`, `delete`, `ip`, `exec` | `RealTartRunner` |
| `git` | `rev-parse`, `worktree list/add` | `RealGitWorktree` |
| `ssh` | check connection, run command, interactive (exec replaces process) | `RealSshClient` |
| `ssh-keygen` | Generate `~/.ssh/tachikoma` ed25519 key | `ensure_tachikoma_key()` |
| `security` | `find-generic-password` for keychain | `try_keychain_entry()` |

### Guest-Side (via `tart exec`)

| Command | Purpose |
|---------|---------|
| `mount -t virtiofs ...` | Mount shared directories |
| `echo <b64> \| base64 -d >> ~/.profile` | Set env vars (GIT_DIR, API keys, TACHIKOMA) |
| `echo <b64> \| base64 -d > ~/.claude/.credentials.json` | Inject OAuth credentials |
| `hostnamectl set-hostname <slug>` | Set VM hostname to branch slug |
| `curl ... \| bash` | Install Claude Code |
| `python3 -c ...` | Patch `~/.claude.json` (hasCompletedOnboarding) |
| `git config --global ...` | Set git identity + safe.directory |
| `ln -sf /mnt/tachikoma/claude-* ~/.claude/` | Symlink host Claude config directories |
| `apt-get install gh` + `echo <b64> \| base64 -d > ~/.config/gh/hosts.yml` | Install gh CLI + inject host auth (if `sync_gh_auth = true`) |

---

## File I/O

### Host Files Read

| Path | Format | Purpose |
|------|--------|---------|
| `~/.config/tachikoma/config.toml` | TOML | Global config |
| `<repo>/.tachikoma.toml` | TOML | Repo config |
| `<repo>/.tachikoma.local.toml` | TOML | Local config |
| `~/.config/tachikoma/state.json` | JSON | VM state |
| `~/.ssh/tachikoma.pub` | text | SSH public key |
| `~/.claude/.credentials.json` | JSON | Credential waterfall source 4 |
| `~/.claude/settings.json` | JSON | Stripped + injected into VM |
| `~/.config/gh/hosts.yml` | YAML | gh CLI auth (if `sync_gh_auth = true`) |
| `**/profiles/*.sh` | shell | Provisioning scripts |

### Host Files Written

| Path | Purpose |
|------|---------|
| `~/.config/tachikoma/state.json` | VM state (atomic: tmp + rename) |
| `~/.config/tachikoma/state.lock` | Advisory lock (fd-lock) |
| `~/.ssh/tachikoma{,.pub}` | SSH key pair (one-time) |
| `~/.ssh/config` | Managed block with VM entries |

### VM Files Written (via `tart exec`)

| Path | Purpose |
|------|---------|
| `~/.ssh/authorized_keys` | Host SSH access |
| `~/.profile` | Env vars (GIT_DIR, API keys, TACHIKOMA=1), cd to code dir |
| `~/.claude/.credentials.json` | MCP OAuth tokens (base64-decoded) |
| `~/.claude/settings.json` | Host settings (stripped of hooks/statusLine/macOS deny rules) |
| `~/.claude/{rules,agents,plugins,skills}` | Symlinks to `/mnt/tachikoma/claude-*/` |
| `~/.claude/projects/<slug>/memory` | Symlink to `/mnt/tachikoma/claude-memory` |
| `~/.claude.json` | Patched with `hasCompletedOnboarding: true` |
| `~/.gitconfig` | user.name, user.email, safe.directory |
| `/etc/hostname` | Branch slug (e.g. `feature-ui-button`) |

### virtiofs Mounts

| Share Name | Host Path | VM Path | Mode |
|-----------|-----------|---------|------|
| `code` | `<worktree>` | `/mnt/tachikoma/code` + `~/code` | read-only |
| `dotgit` | `<repo>/.git` | `/mnt/tachikoma/dotgit` | read-only |
| `claude-rules` | `~/.claude/rules` | `/mnt/tachikoma/claude-rules` -> `~/.claude/rules` | read-only |
| `claude-agents` | `~/.claude/agents` | `/mnt/tachikoma/claude-agents` -> `~/.claude/agents` | read-only |
| `claude-plugins` | `~/.claude/plugins` | `/mnt/tachikoma/claude-plugins` -> `~/.claude/plugins` | read-only |
| `claude-skills` | `~/.claude/skills` | `/mnt/tachikoma/claude-skills` -> `~/.claude/skills` | read-only |
| `claude-memory` | `~/.claude/projects/<slug>/memory` | `/mnt/tachikoma/claude-memory` -> `~/.claude/projects/<slug>/memory` | read-only |

Only non-sensitive subdirectories are mounted individually. Sensitive data (`history.jsonl`, `projects/`, `debug/`, `file-history/`) is never exposed to the VM. The `settings.json` is read from the host, stripped of host-specific fields (`hooks`, `statusLine`, macOS `~/Library/` deny rules), and base64-encoded for transfer, then written as a writable copy in the VM.

---

## State Persistence

The `FileStateStore` uses **advisory file locking** via `fd-lock::RwLock` on `state.lock`. Reads and writes acquire an exclusive lock. I/O runs on `spawn_blocking` to avoid blocking Tokio. Writes are atomic: serialize to `state.json.tmp`, then `rename()` to `state.json`.

## MCP Server

Stdio JSON-RPC 2.0 implementing the Model Context Protocol. Supports `initialize`, `tools/list`, and `tools/call` methods. Exposes `spawn_vm`, `list_vms`, `destroy_vm`, and `exec_in_vm` tools for programmatic VM management by AI assistants.
