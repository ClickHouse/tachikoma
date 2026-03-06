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
) -> Result<SpawnResult> {
    let orch = VmOrchestrator::new(tart, ssh, git, state_store, config);

    // Resolve branch and repo
    let branch = orch.resolve_branch(branch, cwd).await?;
    let (repo_name, repo_root) = orch.resolve_repo(cwd).await?;

    // Ensure worktree exists
    let worktree_path = orch.ensure_worktree(&repo_root, &branch, &repo_name).await?;

    // Spawn or reconnect
    let result = orch
        .spawn(&branch, &repo_name, &worktree_path, &repo_root)
        .await?;

    // Provision if newly created
    if matches!(result, SpawnResult::Created { .. }) {
        provision_vm(tart, ssh, result.ip(), result.name(), &branch, config).await?;
    }

    // SSH in if interactive
    if interactive {
        ssh.connect_interactive(result.ip(), &config.ssh_user)?;
    }

    Ok(result)
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
            Ok(vec![WorktreeInfo {
                path: PathBuf::from("/tmp/myrepo"),
                branch: Some("main".to_string()),
                is_main: true,
            }])
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
        )
        .await
        .unwrap();

        assert!(matches!(result, SpawnResult::Reconnected { .. }));
    }
}
