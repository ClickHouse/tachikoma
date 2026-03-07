pub mod credentials;
pub mod profile;

use std::net::IpAddr;

use crate::config::Config;
use crate::ssh::SshClient;
use crate::tart::TartRunner;
use crate::Result;

use base64::Engine;
use credentials::{resolve_credentials, resolve_supplementary_credentials, CredentialSource};

/// Encode a value as base64, suitable for safe shell injection via `echo <b64> | base64 -d`.
fn b64(data: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(data.as_bytes())
}

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
#[allow(clippy::too_many_arguments)]
pub async fn provision_vm(
    tart: &dyn TartRunner,
    ssh: &dyn SshClient,
    ip: IpAddr,
    vm_name: &str,
    branch: &str,
    repo_root: &std::path::Path,
    config: &Config,
    on_status: &dyn Fn(&str),
) -> Result<()> {
    // 1. Inject host SSH public key so SSH works for subsequent connections
    on_status("Injecting SSH keys...");
    inject_ssh_key(tart, vm_name, &config.ssh_user).await?;

    // 2. Mount virtiofs shares and set up git environment
    on_status("Mounting shared directories...");
    mount_and_configure_git(tart, vm_name, branch).await?;

    // 3. Set TACHIKOMA=1 in shell profile
    on_status("Configuring environment...");
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
    on_status("Injecting credentials...");
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
        let encoded = b64(&supplementary);
        tart_exec(
            tart,
            vm_name,
            &format!("mkdir -p ~/.claude && echo {encoded} | base64 -d > ~/.claude/.credentials.json"),
        )
        .await
        .ok();
        tracing::info!("Injected supplementary credentials (MCP OAuth)");
    }

    // 6. Install Claude, link host config, and complete first-run initialization
    on_status("Installing Claude Code...");
    install_claude(tart, vm_name).await?;
    on_status("Linking host configuration...");
    link_host_claude_config(tart, vm_name, repo_root).await;

    // 7. Run profile scripts
    on_status("Running provisioning scripts...");
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
    on_status("Verifying SSH connectivity...");
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

    // Set VM hostname to branch slug so shell prompt shows admin@<branch>
    let hostname: String = branch
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let hostname = if hostname.len() > 63 { hostname[..63].trim_end_matches('-').to_string() } else { hostname };
    if !hostname.is_empty() {
        let set_hostname = format!(
            "sudo hostnamectl set-hostname {hostname} 2>/dev/null || sudo hostname {hostname} 2>/dev/null || true; \
             grep -q ' {hostname}$' /etc/hosts || echo '127.0.1.1 {hostname}' | sudo tee -a /etc/hosts >/dev/null"
        );
        tart_exec(tart, vm_name, &set_hostname).await.ok();
        tracing::info!("Set VM hostname to '{hostname}'");
    }

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

/// Install Claude Code in the VM and replicate host settings.
async fn install_claude(tart: &dyn TartRunner, vm_name: &str) -> Result<()> {
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

    // Verify installation and complete first-run initialization.
    // Running a non-interactive command creates Claude's runtime state files
    // so the interactive TUI doesn't show a first-run/login screen.
    match tart_exec(tart, vm_name, "~/.local/bin/claude --version").await {
        Ok(version) => tracing::info!("Claude installed: {}", version.trim()),
        Err(e) => tracing::warn!("Claude install verification failed: {e}"),
    }
    // Source ~/.profile to pick up ANTHROPIC_API_KEY, then run a non-interactive
    // command to complete first-run initialization and verify auth works.
    tart_exec(
        tart,
        vm_name,
        "source ~/.profile && ~/.local/bin/claude -p 'respond with ok' --dangerously-skip-permissions 2>/dev/null || true",
    )
    .await
    .ok();

    // Mark onboarding as complete in ~/.claude.json so the interactive TUI
    // doesn't show the welcome/theme-picker wizard on first launch.
    // The claude -p run above creates ~/.claude.json but doesn't set these flags.
    tart_exec(
        tart,
        vm_name,
        r#"python3 -c "
import json, os
p = os.path.expanduser('~/.claude.json')
d = {}
if os.path.exists(p):
    with open(p) as f: d = json.load(f)
d['hasCompletedOnboarding'] = True
d['numStartups'] = d.get('numStartups', 0) + 1
with open(p, 'w') as f: json.dump(d, f)
""#,
    )
    .await
    .ok();

    Ok(())
}

/// Symlink mounted host Claude subdirectories into the VM's ~/.claude.
/// Only safe, non-sensitive directories are mounted (rules, agents, plugins, skills, project memory).
async fn link_host_claude_config(
    tart: &dyn TartRunner,
    vm_name: &str,
    repo_root: &std::path::Path,
) {
    tart_exec(tart, vm_name, "mkdir -p ~/.claude").await.ok();

    // Each subdir is mounted as its own virtiofs share: /mnt/tachikoma/claude-<name>
    for subdir in ["rules", "agents", "plugins", "skills"] {
        let mount = format!("/mnt/tachikoma/claude-{subdir}");
        tart_exec(
            tart,
            vm_name,
            &format!("[ -d {mount} ] && ln -sf {mount} ~/.claude/{subdir}"),
        )
        .await
        .ok();
    }

    // Symlink project memory (MEMORY.md) if mounted.
    // Claude Code stores project data at ~/.claude/projects/<slug>/ where
    // <slug> is the repo root path with / replaced by - (e.g. -Users-rahul-projects-foo).
    let project_slug = repo_root.to_string_lossy().replace('/', "-");
    let project_dir = format!("~/.claude/projects/{project_slug}");
    tart_exec(
        tart,
        vm_name,
        &format!(
            "if [ -d /mnt/tachikoma/claude-memory ]; then \
                mkdir -p {project_dir} && \
                ln -sf /mnt/tachikoma/claude-memory {project_dir}/memory; \
            fi"
        ),
    )
    .await
    .ok();

    // Inject host settings.json (read from host filesystem, not mount, since
    // settings.json contains fields we need to strip before writing).
    inject_host_claude_settings(tart, vm_name).await;

    tracing::info!("Linked host Claude config into VM");
}

/// Read host's ~/.claude/settings.json, strip host-specific fields, inject into VM.
async fn inject_host_claude_settings(tart: &dyn TartRunner, vm_name: &str) {
    let settings_path = match dirs::home_dir() {
        Some(h) => h.join(".claude").join("settings.json"),
        None => return,
    };

    let contents = match tokio::fs::read_to_string(&settings_path).await {
        Ok(c) => c,
        Err(_) => return,
    };

    let cleaned = match serde_json::from_str::<serde_json::Value>(&contents) {
        Ok(mut v) => {
            if let Some(obj) = v.as_object_mut() {
                obj.remove("hooks");
                obj.remove("statusLine");
                if let Some(perms) = obj.get_mut("permissions") {
                    if let Some(deny) = perms.get_mut("deny") {
                        if let Some(arr) = deny.as_array_mut() {
                            arr.retain(|v| {
                                v.as_str()
                                    .map(|s| !s.contains("~/Library/"))
                                    .unwrap_or(true)
                            });
                        }
                    }
                }
            }
            match serde_json::to_string(&v) {
                Ok(s) => s,
                Err(_) => return,
            }
        }
        Err(_) => return,
    };

    let encoded = b64(&cleaned);
    tart_exec(
        tart,
        vm_name,
        &format!("echo {encoded} | base64 -d > ~/.claude/settings.json"),
    )
    .await
    .ok();
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

/// Inject a line into ~/.profile using base64 encoding to avoid shell escaping issues.
async fn inject_profile_line(tart: &dyn TartRunner, vm_name: &str, line: &str) -> Result<()> {
    let encoded = b64(line);
    tart_exec(
        tart,
        vm_name,
        &format!("echo {encoded} | base64 -d >> ~/.profile"),
    )
    .await
    .map(|_| ())
}

/// Validate that an env var name contains only safe characters (A-Z, 0-9, _).
fn is_valid_env_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && !name.as_bytes()[0].is_ascii_digit()
}

async fn inject_credentials(
    tart: &dyn TartRunner,
    vm_name: &str,
    creds: &CredentialSource,
) -> Result<()> {
    match creds {
        CredentialSource::Keychain(key) => {
            inject_profile_line(tart, vm_name, &format!("export ANTHROPIC_API_KEY={key}\n"))
                .await
                .map_err(|e| {
                    crate::TachikomaError::Provision(format!(
                        "Failed to inject keychain API key: {e}"
                    ))
                })?;
        }
        CredentialSource::File(data) => {
            let encoded = b64(data);
            tart_exec(
                tart,
                vm_name,
                &format!("mkdir -p ~/.claude && echo {encoded} | base64 -d > ~/.claude/.credentials.json"),
            )
            .await
            .map_err(|e| {
                crate::TachikomaError::Provision(format!(
                    "Failed to inject credentials: {e}"
                ))
            })?;
        }
        CredentialSource::EnvVar(token) | CredentialSource::Command(token) => {
            inject_profile_line(tart, vm_name, &format!("export CLAUDE_CODE_OAUTH_TOKEN={token}\n"))
                .await
                .map_err(|e| {
                    crate::TachikomaError::Provision(format!(
                        "Failed to inject OAuth token: {e}"
                    ))
                })?;
        }
        CredentialSource::ApiKey(key) | CredentialSource::ApiKeyCommand(key) => {
            inject_profile_line(tart, vm_name, &format!("export ANTHROPIC_API_KEY={key}\n"))
                .await
                .map_err(|e| {
                    crate::TachikomaError::Provision(format!(
                        "Failed to inject API key: {e}"
                    ))
                })?;
        }
        CredentialSource::ProxyEnv { vars, .. } => {
            for (key, value) in vars {
                if !is_valid_env_name(key) {
                    tracing::warn!("Skipping invalid env var name: {key}");
                    continue;
                }
                inject_profile_line(tart, vm_name, &format!("export {key}={value}\n"))
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
    use std::path::Path;

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
        let result = provision_vm(&tart, &ssh, ip, "test-vm", "main", Path::new("/tmp/repo"), &config, &|_| {}).await;
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
        let result = provision_vm(&tart, &ssh, ip, "test-vm", "main", Path::new("/tmp/repo"), &config, &|_| {}).await;
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
        // Credentials are now base64-encoded for shell safety
        assert!(
            cmds.iter().any(|c| c.contains("base64 -d >> ~/.profile")),
            "Expected base64 profile injection, got: {cmds:?}"
        );
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
            cmds.iter().any(|c| c.contains("base64 -d >> ~/.profile")),
            "Expected base64 profile injection, got: {cmds:?}"
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
            cmds.iter().any(|c| c.contains("base64 -d >> ~/.profile")),
            "Expected base64 profile injection, got: {cmds:?}"
        );
    }

    #[test]
    fn test_is_valid_env_name() {
        assert!(is_valid_env_name("AWS_REGION"));
        assert!(is_valid_env_name("ANTHROPIC_API_KEY"));
        assert!(is_valid_env_name("A"));
        assert!(!is_valid_env_name(""));
        assert!(!is_valid_env_name("0START"));
        assert!(!is_valid_env_name("has space"));
        assert!(!is_valid_env_name("lower"));
        assert!(!is_valid_env_name("key=value"));
        assert!(!is_valid_env_name("key;rm -rf /"));
    }

    #[test]
    fn test_b64_roundtrip() {
        let input = "export ANTHROPIC_API_KEY=sk-test'with\"special\nchars\n";
        let encoded = b64(input);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), input);
    }
}
