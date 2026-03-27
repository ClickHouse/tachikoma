use std::path::Path;

use crate::config::Config;
use crate::provision::provision_vm;
use crate::ssh::SshClient;
use crate::state::StateStore;
use crate::tart::TartRunner;
use crate::vm::{SpawnResult, VmOrchestrator};
use crate::worktree::GitWorktree;
use crate::Result;

#[allow(clippy::too_many_arguments)]
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
    let orch = VmOrchestrator::new(tart, ssh, git, state_store, config);

    // Resolve branch and repo
    on_status("Resolving branch...");
    let branch = orch.resolve_branch(branch, cwd).await?;
    let (repo_name, repo_root) = orch.resolve_repo(cwd).await?;

    // Ensure the credential proxy is running before provisioning if enabled
    if config.credential_proxy {
        on_status("Ensuring credential proxy is running...");
        ensure_proxy_running(config).await?;
    }

    // Ensure worktree exists
    on_status("Preparing worktree...");
    let worktree_path = orch
        .ensure_worktree(&repo_root, &branch, &repo_name)
        .await?;

    // Spawn or reconnect
    on_status("Spawning VM...");
    let result = orch
        .spawn(&branch, &repo_name, &worktree_path, &repo_root, on_status)
        .await?;

    // Provision if newly created
    if matches!(result, SpawnResult::Created { .. }) {
        on_status("Provisioning VM...");
        provision_vm(
            tart,
            ssh,
            result.ip(),
            result.name(),
            &branch,
            &repo_root,
            config,
            on_status,
        )
        .await?;
    }

    // SSH in if interactive
    if interactive {
        ssh.connect_interactive(result.ip(), &config.ssh_user)?;
    }

    Ok(result)
}

/// Ensure the credential proxy is running, starting it as a background daemon if needed.
///
/// - If the proxy is already reachable: return immediately.
/// - Otherwise: spawn `tachikoma proxy` as a detached process (setsid) and wait
///   up to 2 s for it to become reachable.
async fn ensure_proxy_running(config: &Config) -> Result<()> {
    let bind = &config.credential_proxy_bind;
    let port = config.credential_proxy_port;

    if crate::proxy::is_proxy_reachable(bind, port).await {
        tracing::debug!("Credential proxy already reachable at {bind}:{port}");
        return Ok(());
    }

    tracing::info!("Starting credential proxy at {bind}:{port}");

    // Resolve the tachikoma binary path (same binary that is running now)
    let exe = std::env::current_exe().map_err(|e| {
        crate::TachikomaError::Proxy(format!("Cannot resolve own executable path: {e}"))
    })?;

    // Spawn detached: call setsid(2) in pre_exec to create a new session so the
    // child survives the parent process exiting. The `setsid` CLI binary is Linux-only
    // but the setsid(2) syscall is POSIX and available on macOS via libc.
    #[cfg(unix)]
    {
        let mut cmd = tokio::process::Command::new(&exe);
        cmd.arg("proxy")
            .arg("start")
            .arg("--port")
            .arg(port.to_string())
            .arg("--bind")
            .arg(bind)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        // Safety: setsid() is async-signal-safe and safe to call in pre_exec.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        cmd.spawn().map_err(|e| {
            crate::TachikomaError::Proxy(format!("Failed to spawn credential proxy: {e}"))
        })?;
    }
    #[cfg(not(unix))]
    tokio::process::Command::new(&exe)
        .arg("proxy")
        .arg("start")
        .arg("--port")
        .arg(port.to_string())
        .arg("--bind")
        .arg(bind)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            crate::TachikomaError::Proxy(format!("Failed to spawn credential proxy: {e}"))
        })?;

    // Wait up to 2 s for the proxy to become reachable
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(2);
    loop {
        if crate::proxy::is_proxy_reachable(bind, port).await {
            tracing::info!("Credential proxy is up at {bind}:{port}");
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!("Credential proxy did not become reachable within 2 s at {bind}:{port}");
            // Non-fatal: provisioning will still inject ANTHROPIC_BASE_URL; the proxy
            // may come up before the VM finishes booting.
            return Ok(());
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PartialConfig;
    use crate::ssh::MockSshClient;
    use crate::state::{MockStateStore, State};
    use crate::tart::types::TartVmInfo;
    use crate::tart::MockTartRunner;
    use crate::worktree::{MockGitWorktree, WorktreeInfo};
    use std::net::{IpAddr, Ipv4Addr};
    use std::path::PathBuf;

    fn test_config() -> Config {
        Config::from_partial(PartialConfig::default()).unwrap()
    }

    #[tokio::test]
    async fn test_spawn_reconnects_running() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));

        let mut tart = MockTartRunner::new();
        let vm_name = crate::vm_name("myrepo", "main");
        let vn = vm_name.clone();
        tart.expect_list().returning(move || {
            Ok(vec![TartVmInfo {
                name: vn.clone(),
                state: "running".to_string(),
                disk: 50,
                size: 31,
                source: "local".to_string(),
                running: true,
                accessed: None,
            }])
        });
        tart.expect_ip().returning(move |_| Ok(Some(ip)));

        let mut ssh = MockSshClient::new();
        ssh.expect_check_connection().returning(|_, _| Ok(true));

        let mut git = MockGitWorktree::new();
        git.expect_current_branch()
            .returning(|_| Ok("main".to_string()));
        git.expect_find_repo_root()
            .returning(|_| Ok(PathBuf::from("/tmp/myrepo")));
        git.expect_list_worktrees().returning(|_| {
            Ok(vec![
                WorktreeInfo {
                    path: PathBuf::from("/tmp/myrepo"),
                    branch: Some("main".to_string()),
                    is_main: true,
                },
                WorktreeInfo {
                    path: PathBuf::from("/tmp/myrepo-main"),
                    branch: Some("main".to_string()),
                    is_main: false,
                },
            ])
        });

        let mut state_store = MockStateStore::new();
        state_store.expect_load().returning(|| Ok(State::new()));
        state_store.expect_save().returning(|_| Ok(()));

        let config = test_config();
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
        .await
        .unwrap();

        assert!(matches!(result, SpawnResult::Reconnected { .. }));
    }
}
