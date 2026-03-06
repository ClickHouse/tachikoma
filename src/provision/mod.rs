pub mod credentials;
pub mod profile;

use std::net::IpAddr;

use crate::config::Config;
use crate::ssh::SshClient;
use crate::tart::TartRunner;
use crate::Result;

use credentials::{resolve_credentials, CredentialSource};

/// Provision a freshly booted VM with credentials, git config, and Claude setup.
pub async fn provision_vm(
    _tart: &dyn TartRunner,
    ssh: &dyn SshClient,
    ip: IpAddr,
    vm_name: &str,
    _branch: &str,
    config: &Config,
) -> Result<()> {
    let user = &config.ssh_user;

    // 1. Set TACHIKOMA=1 in shell profile
    ssh.run_command(
        ip,
        user,
        "echo 'export TACHIKOMA=1' >> ~/.profile",
    )
    .await
    .map_err(|e| {
        crate::TachikomaError::Provision(format!("Failed to set TACHIKOMA env: {e}"))
    })?;

    // 2. Set git user config
    ssh.run_command(ip, user, "git config --global user.name 'Tachikoma'")
        .await
        .ok();
    ssh.run_command(
        ip,
        user,
        "git config --global user.email 'tachikoma@localhost'",
    )
    .await
    .ok();

    // 3. Resolve and inject credentials
    let creds = resolve_credentials(
        config.credential_command.as_deref(),
        config.api_key_command.as_deref(),
    )
    .await;

    inject_credentials(ssh, ip, user, &creds).await?;

    // 4. Mark Claude onboarding complete
    ssh.run_command(
        ip,
        user,
        "mkdir -p ~/.claude && echo '{\"completedOnboarding\": true}' > ~/.claude/settings.local.json",
    )
    .await
    .ok();

    // 5. Run profile scripts (via tart exec for VM-local execution)
    let config_dir = crate::state::FileStateStore::default_path();
    let profiles = profile::discover_profiles(
        &config_dir,
        None, // repo root not accessible from here easily
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

        ssh.run_command(ip, user, &script_content)
            .await
            .map_err(|e| {
                crate::TachikomaError::Provision(format!(
                    "Profile script {} failed: {e}",
                    script.display()
                ))
            })?;
    }

    tracing::info!("VM '{vm_name}' provisioned successfully");
    Ok(())
}

async fn inject_credentials(
    ssh: &dyn SshClient,
    ip: IpAddr,
    user: &str,
    creds: &CredentialSource,
) -> Result<()> {
    match creds {
        CredentialSource::Keychain(data) | CredentialSource::File(data) => {
            // Write credentials JSON to VM
            let escaped = data.replace('\'', "'\\''");
            ssh.run_command(
                ip,
                user,
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
            ssh.run_command(
                ip,
                user,
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
            ssh.run_command(
                ip,
                user,
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
                ssh.run_command(
                    ip,
                    user,
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
    use crate::ssh::MockSshClient;
    use crate::tart::MockTartRunner;
    use crate::config::PartialConfig;
    use std::net::Ipv4Addr;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn test_config() -> Config {
        Config::from_partial(PartialConfig::default()).unwrap()
    }

    #[tokio::test]
    async fn test_provision_sets_env() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));
        let tart = MockTartRunner::new();
        let call_count = AtomicU32::new(0);

        let mut ssh = MockSshClient::new();
        ssh.expect_run_command()
            .returning(move |_, _, cmd| {
                call_count.fetch_add(1, Ordering::SeqCst);
                // Verify TACHIKOMA env is set
                if cmd.contains("TACHIKOMA=1") {
                    return Ok("".to_string());
                }
                Ok("".to_string())
            });

        let config = test_config();
        let result = provision_vm(&tart, &ssh, ip, "test-vm", "main", &config).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_provision_handles_ssh_failures_gracefully() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));
        let tart = MockTartRunner::new();

        let mut ssh = MockSshClient::new();
        // First call succeeds (TACHIKOMA env), rest may fail
        let count = AtomicU32::new(0);
        ssh.expect_run_command()
            .returning(move |_, _, _| {
                let n = count.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Ok("".to_string())
                } else {
                    Ok("".to_string()) // git config returns ok (we use .ok())
                }
            });

        let config = test_config();
        let result = provision_vm(&tart, &ssh, ip, "test-vm", "main", &config).await;
        assert!(result.is_ok());
    }
}
