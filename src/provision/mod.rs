pub mod credentials;
pub mod profile;

use std::net::IpAddr;

use crate::config::Config;
use crate::ssh::SshClient;
use crate::tart::TartRunner;
use crate::Result;

use credentials::{resolve_credentials, resolve_supplementary_credentials, CredentialSource};

/// Run a command in the VM via tart exec, returning Ok/Err.
async fn tart_exec(tart: &dyn TartRunner, vm_name: &str, cmd: &str) -> Result<String> {
    let output = tart
        .exec(
            vm_name,
            vec![
                "bash".to_string(),
                "-c".to_string(),
                cmd.to_string(),
            ],
        )
        .await?;

    if output.exit_code != 0 {
        return Err(crate::TachikomaError::Provision(format!(
            "Command failed (exit {}): {}",
            output.exit_code,
            output.stderr.trim()
        )));
    }

    Ok(output.stdout)
}

/// Provision a freshly booted VM with SSH keys, credentials, git config, and Claude setup.
/// Uses tart exec for initial provisioning (SSH isn't available until keys are injected).
pub async fn provision_vm(
    tart: &dyn TartRunner,
    ssh: &dyn SshClient,
    ip: IpAddr,
    vm_name: &str,
    branch: &str,
    config: &Config,
) -> Result<()> {
    // 1. Inject host SSH public key so SSH works for subsequent connections
    inject_ssh_key(tart, vm_name, &config.ssh_user).await?;

    // 2. Mount virtiofs shares and set up git environment
    mount_and_configure_git(tart, vm_name, branch).await?;

    // 3. Set TACHIKOMA=1 in shell profile
    tart_exec(tart, vm_name, "echo 'export TACHIKOMA=1' >> ~/.profile")
        .await
        .map_err(|e| {
            crate::TachikomaError::Provision(format!("Failed to set TACHIKOMA env: {e}"))
        })?;

    // 4. Set git user config
    tart_exec(tart, vm_name, "git config --global user.name 'Tachikoma'")
        .await
        .ok();
    tart_exec(
        tart,
        vm_name,
        "git config --global user.email 'tachikoma@localhost'",
    )
    .await
    .ok();

    // 5. Resolve and inject credentials
    let creds = resolve_credentials(
        config.credential_command.as_deref(),
        config.api_key_command.as_deref(),
    )
    .await;

    tracing::info!("Credential source: {}", creds.label());
    if creds.is_none() {
        tracing::warn!("No credentials found. Claude will not be able to authenticate in the VM.");
    }

    inject_credentials(tart, vm_name, &creds).await?;

    // 5b. Inject supplementary credentials (MCP OAuth etc.) if available
    if let Some(supplementary) = resolve_supplementary_credentials().await {
        let escaped = supplementary.replace('\'', "'\\''");
        tart_exec(
            tart,
            vm_name,
            &format!("mkdir -p ~/.claude && echo '{escaped}' > ~/.claude/.credentials.json"),
        )
        .await
        .ok();
        tracing::info!("Injected supplementary credentials (MCP OAuth)");
    }

    // 6. Install Claude and mark onboarding complete
    install_claude(tart, vm_name).await?;

    // 7. Run profile scripts
    let config_dir = crate::state::FileStateStore::default_path();
    let profiles = profile::discover_profiles(
        &config_dir,
        None,
        &config.provision_scripts,
    )
    .await?;

    for script in &profiles {
        let script_content = tokio::fs::read_to_string(script).await.map_err(|e| {
            crate::TachikomaError::Provision(format!(
                "Failed to read profile script {}: {e}",
                script.display()
            ))
        })?;

        tart_exec(tart, vm_name, &script_content)
            .await
            .map_err(|e| {
                crate::TachikomaError::Provision(format!(
                    "Profile script {} failed: {e}",
                    script.display()
                ))
            })?;
    }

    // 7. Verify SSH connectivity now works
    let user = &config.ssh_user;
    if let Err(e) = ssh.check_connection(ip, user).await {
        tracing::warn!("SSH verification failed after provisioning: {e}");
    }

    tracing::info!("VM '{vm_name}' provisioned successfully");
    Ok(())
}

/// Mount virtiofs shares inside the VM and configure git environment.
async fn mount_and_configure_git(
    tart: &dyn TartRunner,
    vm_name: &str,
    branch: &str,
) -> Result<()> {
    // Mount the virtiofs automount point
    tart_exec(
        tart,
        vm_name,
        "sudo mkdir -p /mnt/tachikoma && sudo mount -t virtiofs com.apple.virtio-fs.automount /mnt/tachikoma",
    )
    .await
    .map_err(|e| {
        crate::TachikomaError::Provision(format!("Failed to mount virtiofs: {e}"))
    })?;

    // Symlink for convenience
    tart_exec(tart, vm_name, "ln -sf /mnt/tachikoma/code ~/code")
        .await
        .ok();

    // Determine GIT_DIR: for linked worktrees, point at the worktree-specific gitdir
    let git_dir_check = format!(
        "if [ -d /mnt/tachikoma/dotgit/worktrees/{branch} ]; then echo worktree; else echo main; fi"
    );
    let git_type = tart_exec(tart, vm_name, &git_dir_check)
        .await
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "main".to_string());

    let git_dir = if git_type == "worktree" {
        format!("/mnt/tachikoma/dotgit/worktrees/{branch}")
    } else {
        "/mnt/tachikoma/dotgit".to_string()
    };

    // Set up git environment in profile
    let profile_cmds = format!(
        "echo 'export GIT_DIR={git_dir}' >> ~/.profile && \
         echo 'export GIT_WORK_TREE=/mnt/tachikoma/code' >> ~/.profile && \
         echo 'cd /mnt/tachikoma/code' >> ~/.profile"
    );
    tart_exec(tart, vm_name, &profile_cmds)
        .await
        .map_err(|e| {
            crate::TachikomaError::Provision(format!("Failed to set git environment: {e}"))
        })?;

    // Mark the directory as safe for git
    tart_exec(
        tart,
        vm_name,
        "git config --global --add safe.directory /mnt/tachikoma/code",
    )
    .await
    .ok();

    tracing::info!("Mounted virtiofs shares and configured git (GIT_DIR={git_dir})");
    Ok(())
}

/// Install Claude Code in the VM.
async fn install_claude(tart: &dyn TartRunner, vm_name: &str) -> Result<()> {
    // Mark onboarding complete first
    tart_exec(
        tart,
        vm_name,
        "mkdir -p ~/.claude && echo '{\"completedOnboarding\": true}' > ~/.claude/settings.local.json",
    )
    .await
    .ok();

    // Install Claude (script requires bash, not dash/sh)
    tart_exec(
        tart,
        vm_name,
        "curl -fsSL https://claude.ai/install.sh | bash",
    )
    .await
    .map_err(|e| {
        crate::TachikomaError::Provision(format!("Failed to install Claude: {e}"))
    })?;

    // Verify installation (~/.local/bin/claude is the default install location)
    match tart_exec(tart, vm_name, "~/.local/bin/claude --version").await {
        Ok(version) => tracing::info!("Claude installed: {}", version.trim()),
        Err(e) => tracing::warn!("Claude install verification failed: {e}"),
    }

    Ok(())
}

/// Ensure a tachikoma-specific SSH key pair exists on the host, generating one if needed.
async fn ensure_tachikoma_key() -> Result<std::path::PathBuf> {
    let key_path = crate::ssh::tachikoma_key_path().ok_or_else(|| {
        crate::TachikomaError::Provision("Cannot determine home directory".to_string())
    })?;
    let pub_path = key_path.with_extension("pub");

    if !pub_path.exists() {
        tracing::info!("Generating SSH key pair at {}", key_path.display());

        // Ensure ~/.ssh exists with correct permissions
        if let Some(ssh_dir) = key_path.parent() {
            tokio::fs::create_dir_all(ssh_dir).await.map_err(|e| {
                crate::TachikomaError::Provision(format!("Failed to create ~/.ssh: {e}"))
            })?;
        }

        let output = tokio::process::Command::new("ssh-keygen")
            .args([
                "-t", "ed25519",
                "-f", &key_path.display().to_string(),
                "-N", "",
                "-C", "tachikoma",
            ])
            .output()
            .await
            .map_err(|e| {
                crate::TachikomaError::Provision(format!("Failed to run ssh-keygen: {e}"))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::TachikomaError::Provision(format!(
                "ssh-keygen failed: {stderr}"
            )));
        }
    }

    Ok(key_path)
}

/// Inject the host's SSH public key into the VM's authorized_keys.
async fn inject_ssh_key(tart: &dyn TartRunner, vm_name: &str, user: &str) -> Result<()> {
    // Ensure tachikoma key exists (generate if needed), then use it
    let tachikoma_key = ensure_tachikoma_key().await?;
    let pub_path = tachikoma_key.with_extension("pub");

    let pub_key = tokio::fs::read_to_string(&pub_path)
        .await
        .map_err(|e| {
            crate::TachikomaError::Provision(format!(
                "Failed to read tachikoma public key {}: {e}",
                pub_path.display()
            ))
        })?;

    let pub_key = pub_key.trim().to_string();
    if pub_key.is_empty() {
        return Err(crate::TachikomaError::Provision(
            "Tachikoma public key is empty".to_string(),
        ));
    };

    let escaped = pub_key.replace('\'', "'\\''");
    let home_dir = if user == "root" {
        "/root".to_string()
    } else {
        format!("/home/{user}")
    };

    tart_exec(
        tart,
        vm_name,
        &format!(
            "mkdir -p {home_dir}/.ssh && echo '{escaped}' >> {home_dir}/.ssh/authorized_keys && chmod 700 {home_dir}/.ssh && chmod 600 {home_dir}/.ssh/authorized_keys"
        ),
    )
    .await
    .map_err(|e| {
        crate::TachikomaError::Provision(format!("Failed to inject SSH key: {e}"))
    })?;

    Ok(())
}

async fn inject_credentials(
    tart: &dyn TartRunner,
    vm_name: &str,
    creds: &CredentialSource,
) -> Result<()> {
    match creds {
        CredentialSource::Keychain(key) => {
            // Keychain "Claude Code" entry contains an API key
            let escaped = key.replace('\'', "'\\''");
            tart_exec(
                tart,
                vm_name,
                &format!("echo 'export ANTHROPIC_API_KEY={escaped}' >> ~/.profile"),
            )
            .await
            .map_err(|e| {
                crate::TachikomaError::Provision(format!(
                    "Failed to inject keychain API key: {e}"
                ))
            })?;
        }
        CredentialSource::File(data) => {
            let escaped = data.replace('\'', "'\\''");
            tart_exec(
                tart,
                vm_name,
                &format!("mkdir -p ~/.claude && echo '{escaped}' > ~/.claude/.credentials.json"),
            )
            .await
            .map_err(|e| {
                crate::TachikomaError::Provision(format!(
                    "Failed to inject credentials: {e}"
                ))
            })?;
        }
        CredentialSource::EnvVar(token) | CredentialSource::Command(token) => {
            let escaped = token.replace('\'', "'\\''");
            tart_exec(
                tart,
                vm_name,
                &format!("echo 'export CLAUDE_CODE_OAUTH_TOKEN={escaped}' >> ~/.profile"),
            )
            .await
            .map_err(|e| {
                crate::TachikomaError::Provision(format!(
                    "Failed to inject OAuth token: {e}"
                ))
            })?;
        }
        CredentialSource::ApiKey(key) | CredentialSource::ApiKeyCommand(key) => {
            let escaped = key.replace('\'', "'\\''");
            tart_exec(
                tart,
                vm_name,
                &format!("echo 'export ANTHROPIC_API_KEY={escaped}' >> ~/.profile"),
            )
            .await
            .map_err(|e| {
                crate::TachikomaError::Provision(format!(
                    "Failed to inject API key: {e}"
                ))
            })?;
        }
        CredentialSource::ProxyEnv { vars, .. } => {
            for (key, value) in vars {
                let escaped_val = value.replace('\'', "'\\''");
                tart_exec(
                    tart,
                    vm_name,
                    &format!("echo 'export {key}={escaped_val}' >> ~/.profile"),
                )
                .await
                .ok();
            }
        }
        CredentialSource::None => {
            tracing::warn!("No credentials found. Claude may not authenticate in the VM.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PartialConfig;
    use crate::ssh::MockSshClient;
    use crate::tart::types::ExecOutput;
    use crate::tart::MockTartRunner;
    use std::net::Ipv4Addr;

    fn test_config() -> Config {
        Config::from_partial(PartialConfig::default()).unwrap()
    }

    fn mock_tart_exec_ok() -> MockTartRunner {
        let mut tart = MockTartRunner::new();
        tart.expect_exec().returning(|_, _| {
            Ok(ExecOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            })
        });
        tart
    }

    #[tokio::test]
    async fn test_provision_sets_env() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));
        let tart = mock_tart_exec_ok();

        let mut ssh = MockSshClient::new();
        ssh.expect_check_connection().returning(|_, _| Ok(true));

        let config = test_config();
        let result = provision_vm(&tart, &ssh, ip, "test-vm", "main", &config).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_provision_handles_ssh_verify_failure() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));
        let tart = mock_tart_exec_ok();

        let mut ssh = MockSshClient::new();
        // SSH verify fails but provisioning still succeeds
        ssh.expect_check_connection()
            .returning(|_, _| Err(crate::TachikomaError::Ssh("connection refused".into())));

        let config = test_config();
        let result = provision_vm(&tart, &ssh, ip, "test-vm", "main", &config).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_inject_keychain_sets_api_key_env() {
        use std::sync::{Arc, Mutex};

        let commands: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let cmds = commands.clone();

        let mut tart = MockTartRunner::new();
        tart.expect_exec().returning(move |_, args| {
            if args.len() >= 3 {
                cmds.lock().unwrap().push(args[2].clone());
            }
            Ok(ExecOutput { stdout: String::new(), stderr: String::new(), exit_code: 0 })
        });

        let creds = CredentialSource::Keychain("sk-ant-test-key".into());
        inject_credentials(&tart, "test-vm", &creds).await.unwrap();

        let cmds = commands.lock().unwrap();
        assert!(
            cmds.iter().any(|c| c.contains("ANTHROPIC_API_KEY=sk-ant-test-key")),
            "Expected ANTHROPIC_API_KEY in profile, got: {cmds:?}"
        );
        // Should NOT write to .credentials.json
        assert!(
            !cmds.iter().any(|c| c.contains(".credentials.json")),
            "Keychain API key should not be written to .credentials.json"
        );
    }

    #[tokio::test]
    async fn test_inject_file_writes_credentials_json() {
        use std::sync::{Arc, Mutex};

        let commands: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let cmds = commands.clone();

        let mut tart = MockTartRunner::new();
        tart.expect_exec().returning(move |_, args| {
            if args.len() >= 3 {
                cmds.lock().unwrap().push(args[2].clone());
            }
            Ok(ExecOutput { stdout: String::new(), stderr: String::new(), exit_code: 0 })
        });

        let creds = CredentialSource::File(r#"{"oauth":"token"}"#.into());
        inject_credentials(&tart, "test-vm", &creds).await.unwrap();

        let cmds = commands.lock().unwrap();
        assert!(
            cmds.iter().any(|c| c.contains(".credentials.json")),
            "File creds should write to .credentials.json, got: {cmds:?}"
        );
    }

    #[tokio::test]
    async fn test_inject_api_key_sets_env() {
        use std::sync::{Arc, Mutex};

        let commands: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let cmds = commands.clone();

        let mut tart = MockTartRunner::new();
        tart.expect_exec().returning(move |_, args| {
            if args.len() >= 3 {
                cmds.lock().unwrap().push(args[2].clone());
            }
            Ok(ExecOutput { stdout: String::new(), stderr: String::new(), exit_code: 0 })
        });

        let creds = CredentialSource::ApiKey("sk-test-123".into());
        inject_credentials(&tart, "test-vm", &creds).await.unwrap();

        let cmds = commands.lock().unwrap();
        assert!(
            cmds.iter().any(|c| c.contains("ANTHROPIC_API_KEY=sk-test-123")),
            "Expected ANTHROPIC_API_KEY env, got: {cmds:?}"
        );
    }

    #[tokio::test]
    async fn test_inject_oauth_token_sets_env() {
        use std::sync::{Arc, Mutex};

        let commands: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let cmds = commands.clone();

        let mut tart = MockTartRunner::new();
        tart.expect_exec().returning(move |_, args| {
            if args.len() >= 3 {
                cmds.lock().unwrap().push(args[2].clone());
            }
            Ok(ExecOutput { stdout: String::new(), stderr: String::new(), exit_code: 0 })
        });

        let creds = CredentialSource::EnvVar("oauth-token-123".into());
        inject_credentials(&tart, "test-vm", &creds).await.unwrap();

        let cmds = commands.lock().unwrap();
        assert!(
            cmds.iter().any(|c| c.contains("CLAUDE_CODE_OAUTH_TOKEN=oauth-token-123")),
            "Expected CLAUDE_CODE_OAUTH_TOKEN env, got: {cmds:?}"
        );
    }
}
