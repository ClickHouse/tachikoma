pub mod credentials;
pub mod profile;

use std::net::IpAddr;

use crate::config::Config;
use crate::ssh::SshClient;
use crate::tart::TartRunner;
use crate::Result;

use credentials::{resolve_credentials, CredentialSource};

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
    _branch: &str,
    config: &Config,
) -> Result<()> {
    // 1. Inject host SSH public key so SSH works for subsequent connections
    inject_ssh_key(tart, vm_name, &config.ssh_user).await?;

    // 2. Set TACHIKOMA=1 in shell profile
    tart_exec(tart, vm_name, "echo 'export TACHIKOMA=1' >> ~/.profile")
        .await
        .map_err(|e| {
            crate::TachikomaError::Provision(format!("Failed to set TACHIKOMA env: {e}"))
        })?;

    // 3. Set git user config
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

    // 4. Resolve and inject credentials
    let creds = resolve_credentials(
        config.credential_command.as_deref(),
        config.api_key_command.as_deref(),
    )
    .await;

    inject_credentials(tart, vm_name, &creds).await?;

    // 5. Mark Claude onboarding complete
    tart_exec(
        tart,
        vm_name,
        "mkdir -p ~/.claude && echo '{\"completedOnboarding\": true}' > ~/.claude/settings.local.json",
    )
    .await
    .ok();

    // 6. Run profile scripts
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

/// Inject the host's SSH public key into the VM's authorized_keys.
async fn inject_ssh_key(tart: &dyn TartRunner, vm_name: &str, user: &str) -> Result<()> {
    // Find the host's SSH public key
    let home = dirs::home_dir().ok_or_else(|| {
        crate::TachikomaError::Provision("Cannot determine home directory".to_string())
    })?;

    let key_candidates = [
        home.join(".ssh/id_ed25519.pub"),
        home.join(".ssh/id_rsa.pub"),
        home.join(".ssh/id_ecdsa.pub"),
    ];

    let pub_key = {
        let mut found = None;
        for path in &key_candidates {
            if let Ok(content) = tokio::fs::read_to_string(path).await {
                let trimmed = content.trim().to_string();
                if !trimmed.is_empty() {
                    found = Some(trimmed);
                    break;
                }
            }
        }
        found
    };

    let Some(pub_key) = pub_key else {
        tracing::warn!("No SSH public key found on host. SSH access may not work.");
        return Ok(());
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
        CredentialSource::Keychain(data) | CredentialSource::File(data) => {
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
}
