# Writable Mounts + `tachikoma pr` Design

## Goal

Enable Claude to write files in the VM and surface those changes as a GitHub PR with a single host-side command.

## Problem

The `code` virtiofs mount is currently read-only (`:ro`). Claude can read the repo inside the VM but cannot write changes back. This makes the tool useful only for read/advise workflows, not autonomous coding.

## Design

### Part 1: Writable code mount

In `src/vm/mod.rs` `build_run_opts()`, change the `code` DirMount from `read_only: true` to `read_only: false`. The `.git` mount stays read-only — Claude writes working files but cannot corrupt git internals.

```rust
DirMount {
    name: Some("code".into()),
    host_path: worktree_path.to_path_buf(),
    read_only: false,   // was true
}
```

No provisioning changes required. The virtiofs mount at `/mnt/tachikoma/code` and `~/code` symlink already exist; making the mount writable just removes `:ro` from the tart CLI arg.

### Part 2: `tachikoma pr [--name <vm-name>]`

New CLI subcommand that commits whatever Claude wrote and opens a GitHub PR, all from the host.

**Flow:**

1. Resolve VM name from current branch + repo (or explicit `--name`)
2. Look up worktree path from state store
3. Run `git -C <worktree> diff --stat HEAD` — if empty, print "Nothing to commit" and exit cleanly
4. Build commit message:
   - Subject: `chore: Claude changes on <branch>`
   - Body: full `git diff --stat HEAD` output
5. `git -C <worktree> add -A`
6. `git -C <worktree> commit -m "<message>"`
7. `git -C <worktree> push -u origin <branch>`
8. `gh pr create --fill --head <branch>` — uses commit message as PR title+body
9. Print the PR URL

**Edge cases:**

- No changes → clear message, no empty commit, exit 0
- `gh` not on PATH → "Install gh CLI to create PRs, or push and open manually"
- Push fails → error propagated; commit is already made so changes are not lost
- VM not in state store → same `Vm("not found")` error as `halt`/`destroy`

## Intended workflow

```
tachikoma              # spawn VM, Claude starts working
tachikoma enter        # SSH in, watch / direct Claude
tachikoma pr           # when done: commit + push + open PR
```

## Files to change

| File | Change |
|------|--------|
| `src/vm/mod.rs` | `read_only: false` for `code` DirMount |
| `src/cmd/pr.rs` | New: PR command implementation |
| `src/cmd/mod.rs` | `pub mod pr;` |
| `src/cli/mod.rs` | Add `Pr { name: Option<String> }` variant to `Command` |
| `src/main.rs` | Wire `Command::Pr` to `cmd::pr::run()` |

## Testing

- `src/vm/mod.rs` — update existing `build_run_opts` test: `code` mount `read_only == false`, `.git` mount `read_only == true`
- `src/cmd/pr.rs` — unit tests with mocked state store and git:
  - `test_pr_no_changes` — empty diff stat → `NothingToCommit`
  - `test_pr_creates_commit` — verifies add/commit/push call order
  - `test_pr_vm_not_found` — state has no entry → `Vm("not found")`
- `gh pr create` not unit-tested (not behind trait); integration-tested manually
