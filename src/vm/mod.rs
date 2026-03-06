pub mod boot;

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::config::Config;
use crate::ssh::SshClient;
use crate::state::{StateStore, VmEntry, VmStatus};
use crate::tart::{DirMount, RunOpts, TartRunner, TartVmState};
use crate::worktree::GitWorktree;
use crate::Result;

use boot::{wait_for_boot, BootConfig};

/// Result of the spawn/connect orchestration
#[derive(Debug)]
pub enum SpawnResult {
    /// Connected to an existing running VM
    Reconnected { name: String, ip: IpAddr },
    /// Resumed a suspended VM
    Resumed { name: String, ip: IpAddr },
    /// Started a stopped VM
    Started { name: String, ip: IpAddr },
    /// Created and booted a new VM
    Created { name: String, ip: IpAddr },
}

impl SpawnResult {
    pub fn name(&self) -> &str {
        match self {
            Self::Reconnected { name, .. }
            | Self::Resumed { name, .. }
            | Self::Started { name, .. }
            | Self::Created { name, .. } => name,
        }
    }

    pub fn ip(&self) -> IpAddr {
        match self {
            Self::Reconnected { ip, .. }
            | Self::Resumed { ip, .. }
            | Self::Started { ip, .. }
            | Self::Created { ip, .. } => *ip,
        }
    }
}

/// The core orchestrator that implements the zero-arg state machine.
pub struct VmOrchestrator<'a> {
    tart: &'a dyn TartRunner,
    ssh: &'a dyn SshClient,
    git: &'a dyn GitWorktree,
    state_store: &'a dyn StateStore,
    config: &'a Config,
}

impl<'a> VmOrchestrator<'a> {
    pub fn new(
        tart: &'a dyn TartRunner,
        ssh: &'a dyn SshClient,
        git: &'a dyn GitWorktree,
        state_store: &'a dyn StateStore,
        config: &'a Config,
    ) -> Self {
        Self {
            tart,
            ssh,
            git,
            state_store,
            config,
        }
    }

    /// Resolve branch name: use explicit or detect from current directory
    pub async fn resolve_branch(
        &self,
        explicit: Option<&str>,
        cwd: &Path,
    ) -> Result<String> {
        match explicit {
            Some(branch) => Ok(branch.to_string()),
            None => self.git.current_branch(cwd).await,
        }
    }

    /// Resolve repo name from current directory
    pub async fn resolve_repo(&self, cwd: &Path) -> Result<(String, PathBuf)> {
        let root = self.git.find_repo_root(cwd).await?;
        let repo_name = root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        Ok((repo_name, root))
    }

    /// Find or create a linked worktree for the given branch
    pub async fn ensure_worktree(
        &self,
        repo_root: &Path,
        branch: &str,
        repo_name: &str,
    ) -> Result<PathBuf> {
        let worktrees = self.git.list_worktrees(repo_root).await?;

        // Check if a worktree already exists for this branch
        for wt in &worktrees {
            if wt.branch.as_deref() == Some(branch) {
                tracing::debug!("Found existing worktree for branch '{branch}' at {:?}", wt.path);
                return Ok(wt.path.clone());
            }
        }

        // Create a new worktree
        let base_dir = self.config.worktree_dir.as_deref().unwrap_or_else(|| {
            repo_root.parent().unwrap_or(repo_root)
        });
        let target = base_dir.join(format!("{repo_name}-{branch}"));
        tracing::info!("Creating worktree for branch '{branch}' at {}", target.display());
        self.git.create_worktree(repo_root, branch, &target).await
    }

    /// The main spawn/connect state machine
    pub async fn spawn(
        &self,
        branch: &str,
        repo_name: &str,
        worktree_path: &Path,
        repo_root: &Path,
    ) -> Result<SpawnResult> {
        let vm_name = crate::vm_name(repo_name, branch);
        tracing::info!("VM name: {vm_name}");

        // Check tart's actual VM state
        let tart_vms = self.tart.list().await?;
        let tart_state = tart_vms
            .iter()
            .find(|vm| vm.name == vm_name)
            .map(|vm| vm.state_enum());

        match tart_state {
            Some(TartVmState::Running) => {
                // Already running — get IP and verify SSH
                tracing::info!("VM '{vm_name}' is already running");
                let ip = self.get_ip_or_wait(&vm_name).await?;
                self.update_state(&vm_name, repo_name, branch, worktree_path, VmStatus::Running, Some(ip))
                    .await?;
                Ok(SpawnResult::Reconnected { name: vm_name, ip })
            }
            Some(TartVmState::Suspended) => {
                // Suspended — resume
                tracing::info!("Resuming suspended VM '{vm_name}'");
                let opts = self.build_run_opts(worktree_path, repo_root);
                self.tart.run(&vm_name, &opts).await?;
                let ip = self.wait_boot(&vm_name).await?;
                self.update_state(&vm_name, repo_name, branch, worktree_path, VmStatus::Running, Some(ip))
                    .await?;
                Ok(SpawnResult::Resumed { name: vm_name, ip })
            }
            Some(TartVmState::Stopped) => {
                // Stopped — start
                tracing::info!("Starting stopped VM '{vm_name}'");
                let opts = self.build_run_opts(worktree_path, repo_root);
                self.tart.run(&vm_name, &opts).await?;
                let ip = self.wait_boot(&vm_name).await?;
                self.update_state(&vm_name, repo_name, branch, worktree_path, VmStatus::Running, Some(ip))
                    .await?;
                Ok(SpawnResult::Started { name: vm_name, ip })
            }
            Some(TartVmState::Unknown) | None => {
                // Not found — clone and create
                tracing::info!("Creating new VM '{vm_name}' from '{}'", self.config.base_image);
                self.tart
                    .clone_vm(&self.config.base_image, &vm_name)
                    .await?;
                let opts = self.build_run_opts(worktree_path, repo_root);
                self.tart.run(&vm_name, &opts).await?;
                let ip = self.wait_boot(&vm_name).await?;
                self.update_state(&vm_name, repo_name, branch, worktree_path, VmStatus::Running, Some(ip))
                    .await?;
                Ok(SpawnResult::Created { name: vm_name, ip })
            }
        }
    }

    fn build_run_opts(&self, worktree_path: &Path, repo_root: &Path) -> RunOpts {
        let mut dirs = vec![
            DirMount {
                host_path: worktree_path.to_path_buf(),
                read_only: true,
            },
        ];

        // Mount .git directory read-only
        let git_dir = repo_root.join(".git");
        if git_dir.exists() {
            dirs.push(DirMount {
                host_path: git_dir,
                read_only: true,
            });
        }

        RunOpts {
            no_graphics: self.config.vm_display == "none",
            dirs,
            rosetta: false,
        }
    }

    async fn get_ip_or_wait(&self, vm_name: &str) -> Result<IpAddr> {
        // Try getting IP directly first
        if let Ok(Some(ip)) = self.tart.ip(vm_name).await {
            if self.ssh.check_connection(ip, &self.config.ssh_user).await.unwrap_or(false) {
                return Ok(ip);
            }
        }
        self.wait_boot(vm_name).await
    }

    async fn wait_boot(&self, vm_name: &str) -> Result<IpAddr> {
        let boot_config = BootConfig {
            timeout: std::time::Duration::from_secs(self.config.boot_timeout_secs),
            ssh_user: self.config.ssh_user.clone(),
            ..Default::default()
        };
        wait_for_boot(self.tart, self.ssh, vm_name, &boot_config).await
    }

    async fn update_state(
        &self,
        vm_name: &str,
        repo_name: &str,
        branch: &str,
        worktree_path: &Path,
        status: VmStatus,
        ip: Option<IpAddr>,
    ) -> Result<()> {
        let mut state = self.state_store.load().await?;

        match state.find_vm_mut(vm_name) {
            Some(entry) => {
                entry.status = status;
                entry.ip = ip.map(|i| i.to_string());
                entry.last_used = Utc::now();
            }
            None => {
                state.add_vm(VmEntry {
                    name: vm_name.to_string(),
                    repo: repo_name.to_string(),
                    branch: branch.to_string(),
                    worktree_path: worktree_path.to_path_buf(),
                    created_at: Utc::now(),
                    last_used: Utc::now(),
                    status,
                    ip: ip.map(|i| i.to_string()),
                });
            }
        }

        self.state_store.save(&state).await
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
    use crate::worktree::MockGitWorktree;
    use std::net::Ipv4Addr;

    fn test_config() -> Config {
        Config::from_partial(PartialConfig::default()).unwrap()
    }

    fn test_vm_info(name: &str, state: &str) -> TartVmInfo {
        TartVmInfo {
            name: name.to_string(),
            state: state.to_string(),
            disk: 50,
            size: 31,
            source: "local".to_string(),
            running: state == "running",
            accessed: None,
        }
    }

    fn running_vm(name: &str) -> TartVmInfo {
        test_vm_info(name, "running")
    }

    fn stopped_vm(name: &str) -> TartVmInfo {
        test_vm_info(name, "stopped")
    }

    fn suspended_vm(name: &str) -> TartVmInfo {
        test_vm_info(name, "suspended")
    }

    #[tokio::test]
    async fn test_reconnect_running_vm() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));
        let vm_name = crate::vm_name("myrepo", "main");

        let mut tart = MockTartRunner::new();
        tart.expect_list()
            .returning(move || Ok(vec![running_vm(&crate::vm_name("myrepo", "main"))]));
        tart.expect_ip().returning(move |_| Ok(Some(ip)));

        let mut ssh = MockSshClient::new();
        ssh.expect_check_connection().returning(|_, _| Ok(true));

        let git = MockGitWorktree::new();

        let mut state_store = MockStateStore::new();
        state_store.expect_load().returning(|| Ok(State::new()));
        state_store.expect_save().returning(|_| Ok(()));

        let config = test_config();
        let orch = VmOrchestrator::new(&tart, &ssh, &git, &state_store, &config);

        let result = orch
            .spawn("main", "myrepo", Path::new("/tmp/wt"), Path::new("/tmp/repo"))
            .await
            .unwrap();

        assert!(matches!(result, SpawnResult::Reconnected { .. }));
        assert_eq!(result.name(), vm_name);
        assert_eq!(result.ip(), ip);
    }

    #[tokio::test]
    async fn test_create_new_vm() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));

        let mut tart = MockTartRunner::new();
        tart.expect_list().returning(|| Ok(vec![]));
        tart.expect_clone_vm().returning(|_, _| Ok(()));
        tart.expect_run().returning(|_, _| Ok(()));
        tart.expect_ip().returning(move |_| Ok(Some(ip)));

        let mut ssh = MockSshClient::new();
        ssh.expect_check_connection().returning(|_, _| Ok(true));

        let git = MockGitWorktree::new();

        let mut state_store = MockStateStore::new();
        state_store.expect_load().returning(|| Ok(State::new()));
        state_store.expect_save().returning(|_| Ok(()));

        let config = test_config();
        let orch = VmOrchestrator::new(&tart, &ssh, &git, &state_store, &config);

        let result = orch
            .spawn("main", "myrepo", Path::new("/tmp/wt"), Path::new("/tmp/repo"))
            .await
            .unwrap();

        assert!(matches!(result, SpawnResult::Created { .. }));
    }

    #[tokio::test]
    async fn test_resume_suspended_vm() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));
        let vm_name_str = crate::vm_name("myrepo", "feat");

        let mut tart = MockTartRunner::new();
        let vn = vm_name_str.clone();
        tart.expect_list()
            .returning(move || Ok(vec![suspended_vm(&vn)]));
        tart.expect_run().returning(|_, _| Ok(()));
        tart.expect_ip().returning(move |_| Ok(Some(ip)));

        let mut ssh = MockSshClient::new();
        ssh.expect_check_connection().returning(|_, _| Ok(true));

        let git = MockGitWorktree::new();

        let mut state_store = MockStateStore::new();
        state_store.expect_load().returning(|| Ok(State::new()));
        state_store.expect_save().returning(|_| Ok(()));

        let config = test_config();
        let orch = VmOrchestrator::new(&tart, &ssh, &git, &state_store, &config);

        let result = orch
            .spawn("feat", "myrepo", Path::new("/tmp/wt"), Path::new("/tmp/repo"))
            .await
            .unwrap();

        assert!(matches!(result, SpawnResult::Resumed { .. }));
    }

    #[tokio::test]
    async fn test_start_stopped_vm() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));
        let vm_name_str = crate::vm_name("myrepo", "dev");

        let mut tart = MockTartRunner::new();
        let vn = vm_name_str.clone();
        tart.expect_list()
            .returning(move || Ok(vec![stopped_vm(&vn)]));
        tart.expect_run().returning(|_, _| Ok(()));
        tart.expect_ip().returning(move |_| Ok(Some(ip)));

        let mut ssh = MockSshClient::new();
        ssh.expect_check_connection().returning(|_, _| Ok(true));

        let git = MockGitWorktree::new();

        let mut state_store = MockStateStore::new();
        state_store.expect_load().returning(|| Ok(State::new()));
        state_store.expect_save().returning(|_| Ok(()));

        let config = test_config();
        let orch = VmOrchestrator::new(&tart, &ssh, &git, &state_store, &config);

        let result = orch
            .spawn("dev", "myrepo", Path::new("/tmp/wt"), Path::new("/tmp/repo"))
            .await
            .unwrap();

        assert!(matches!(result, SpawnResult::Started { .. }));
    }

    #[tokio::test]
    async fn test_resolve_explicit_branch() {
        let tart = MockTartRunner::new();
        let ssh = MockSshClient::new();
        let git = MockGitWorktree::new();
        let state_store = MockStateStore::new();
        let config = test_config();

        let orch = VmOrchestrator::new(&tart, &ssh, &git, &state_store, &config);
        let branch = orch.resolve_branch(Some("feature/x"), Path::new("/tmp")).await.unwrap();
        assert_eq!(branch, "feature/x");
    }

    #[tokio::test]
    async fn test_resolve_branch_auto_detect() {
        let tart = MockTartRunner::new();
        let ssh = MockSshClient::new();

        let mut git = MockGitWorktree::new();
        git.expect_current_branch()
            .returning(|_| Ok("main".to_string()));

        let state_store = MockStateStore::new();
        let config = test_config();

        let orch = VmOrchestrator::new(&tart, &ssh, &git, &state_store, &config);
        let branch = orch.resolve_branch(None, Path::new("/tmp")).await.unwrap();
        assert_eq!(branch, "main");
    }

    #[tokio::test]
    async fn test_state_updated_on_create() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));

        let mut tart = MockTartRunner::new();
        tart.expect_list().returning(|| Ok(vec![]));
        tart.expect_clone_vm().returning(|_, _| Ok(()));
        tart.expect_run().returning(|_, _| Ok(()));
        tart.expect_ip().returning(move |_| Ok(Some(ip)));

        let mut ssh = MockSshClient::new();
        ssh.expect_check_connection().returning(|_, _| Ok(true));

        let git = MockGitWorktree::new();

        let mut state_store = MockStateStore::new();
        state_store.expect_load().returning(|| Ok(State::new()));
        state_store
            .expect_save()
            .withf(|state: &State| {
                state.vms.len() == 1 && state.vms[0].status == VmStatus::Running
            })
            .returning(|_| Ok(()));

        let config = test_config();
        let orch = VmOrchestrator::new(&tart, &ssh, &git, &state_store, &config);

        let result = orch
            .spawn("main", "myrepo", Path::new("/tmp/wt"), Path::new("/tmp/repo"))
            .await;
        assert!(result.is_ok());
    }
}
