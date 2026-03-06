use async_trait::async_trait;
use std::net::IpAddr;

use crate::Result;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait SshClient: Send + Sync {
    async fn check_connection(&self, ip: IpAddr, user: &str) -> Result<bool>;
    async fn run_command(&self, ip: IpAddr, user: &str, cmd: &str) -> Result<String>;
    fn connect_interactive(&self, ip: IpAddr, user: &str) -> Result<()>;
}

#[derive(Default)]
pub struct RealSshClient;

impl RealSshClient {
    pub fn new() -> Self {
        Self
    }

    fn ssh_opts() -> Vec<&'static str> {
        vec![
            "-o", "StrictHostKeyChecking=no",
            "-o", "UserKnownHostsFile=/dev/null",
            "-o", "LogLevel=ERROR",
        ]
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
        cmd.arg(format!("{user}@{ip}"));

        // exec() replaces the current process
        let err = cmd.exec();
        Err(crate::TachikomaError::Ssh(format!(
            "Failed to exec ssh: {err}"
        )))
    }
}
