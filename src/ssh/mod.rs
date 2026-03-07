use async_trait::async_trait;
use std::net::IpAddr;
use std::path::PathBuf;

use crate::Result;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait SshClient: Send + Sync {
    async fn check_connection(&self, ip: IpAddr, user: &str) -> Result<bool>;
    async fn check_port_open(&self, ip: IpAddr) -> Result<bool>;
    async fn run_command(&self, ip: IpAddr, user: &str, cmd: &str) -> Result<String>;
    fn connect_interactive(&self, ip: IpAddr, user: &str) -> Result<()>;
}

#[derive(Default)]
pub struct RealSshClient;

/// Path to the tachikoma-specific SSH key pair.
pub fn tachikoma_key_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".ssh/tachikoma"))
}

impl RealSshClient {
    pub fn new() -> Self {
        Self
    }

    fn ssh_opts() -> Vec<String> {
        let mut opts = vec![
            "-o".to_string(), "StrictHostKeyChecking=no".to_string(),
            "-o".to_string(), "UserKnownHostsFile=/dev/null".to_string(),
            "-o".to_string(), "LogLevel=ERROR".to_string(),
        ];
        if let Some(key_path) = tachikoma_key_path() {
            if key_path.exists() {
                opts.push("-i".to_string());
                opts.push(key_path.display().to_string());
            }
        }
        opts
    }
}

#[async_trait]
impl SshClient for RealSshClient {
    async fn check_connection(&self, ip: IpAddr, user: &str) -> Result<bool> {
        let output = tokio::process::Command::new("ssh")
            .args(Self::ssh_opts())
            .args(["-o", "ConnectTimeout=2", "-o", "BatchMode=yes"])
            .arg(format!("{user}@{ip}"))
            .arg("true")
            .output()
            .await
            .map_err(|e| crate::TachikomaError::Ssh(format!("Failed to run ssh: {e}")))?;

        Ok(output.status.success())
    }

    async fn check_port_open(&self, ip: IpAddr) -> Result<bool> {
        use tokio::net::TcpStream;
        match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            TcpStream::connect((ip, 22u16)),
        )
        .await
        {
            Ok(Ok(_)) => Ok(true),
            _ => Ok(false),
        }
    }

    async fn run_command(&self, ip: IpAddr, user: &str, cmd: &str) -> Result<String> {
        let output = tokio::process::Command::new("ssh")
            .args(Self::ssh_opts())
            .args(["-o", "ConnectTimeout=5"])
            .arg(format!("{user}@{ip}"))
            .arg(cmd)
            .output()
            .await
            .map_err(|e| crate::TachikomaError::Ssh(format!("Failed to run ssh command: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::TachikomaError::Ssh(format!(
                "SSH command failed: {stderr}"
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn connect_interactive(&self, ip: IpAddr, user: &str) -> Result<()> {
        use std::os::unix::process::CommandExt;

        let mut cmd = std::process::Command::new("ssh");
        cmd.args(Self::ssh_opts());
        cmd.arg("-t");
        cmd.arg(format!("{user}@{ip}"));

        // exec() replaces the current process
        let err = cmd.exec();
        Err(crate::TachikomaError::Ssh(format!(
            "Failed to exec ssh: {err}"
        )))
    }
}
