pub mod types;

pub use types::{DirMount, ExecOutput, RunOpts, TartVmInfo, TartVmState};

use async_trait::async_trait;
use std::net::IpAddr;
use std::os::unix::process::CommandExt;
use std::process::Stdio;

use crate::Result;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TartRunner: Send + Sync {
    async fn list(&self) -> Result<Vec<TartVmInfo>>;
    async fn clone_vm(&self, source: &str, name: &str) -> Result<()>;
    async fn run(&self, name: &str, opts: &RunOpts) -> Result<()>;
    async fn stop(&self, name: &str) -> Result<()>;
    async fn suspend(&self, name: &str) -> Result<()>;
    async fn delete(&self, name: &str) -> Result<()>;
    async fn ip(&self, name: &str) -> Result<Option<IpAddr>>;
    async fn exec(&self, name: &str, cmd: Vec<String>) -> Result<ExecOutput>;
}

#[derive(Default)]
pub struct RealTartRunner;

impl RealTartRunner {
    pub fn new() -> Self {
        Self
    }

    fn tart_cmd() -> tokio::process::Command {
        tokio::process::Command::new("tart")
    }
}

#[async_trait]
impl TartRunner for RealTartRunner {
    async fn list(&self) -> Result<Vec<TartVmInfo>> {
        let output = Self::tart_cmd()
            .args(["list", "--format", "json"])
            .output()
            .await
            .map_err(|e| crate::TachikomaError::Tart(format!("Failed to run tart list: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::TachikomaError::Tart(format!(
                "tart list failed: {stderr}"
            )));
        }

        let vms: Vec<TartVmInfo> = serde_json::from_slice(&output.stdout).map_err(|e| {
            crate::TachikomaError::Tart(format!("Failed to parse tart list output: {e}"))
        })?;

        Ok(vms)
    }

    async fn clone_vm(&self, source: &str, name: &str) -> Result<()> {
        let output = Self::tart_cmd()
            .args(["clone", source, name])
            .output()
            .await
            .map_err(|e| crate::TachikomaError::Tart(format!("Failed to run tart clone: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::TachikomaError::Tart(format!(
                "tart clone failed: {stderr}"
            )));
        }

        Ok(())
    }

    async fn run(&self, name: &str, opts: &RunOpts) -> Result<()> {
        let mut args = vec!["run".to_string()];

        if opts.no_graphics {
            args.push("--no-graphics".to_string());
        }

        for dir in &opts.dirs {
            args.push("--dir".to_string());
            args.push(dir.to_tart_arg());
        }

        if opts.rosetta {
            args.push("--rosetta".to_string());
        }

        args.push(name.to_string());

        // Spawn tart run as a detached process using setsid()
        let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let mut cmd = std::process::Command::new("tart");
        cmd.args(&args_ref);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }

        cmd.spawn().map_err(|e| {
            crate::TachikomaError::Tart(format!("Failed to spawn tart run: {e}"))
        })?;

        Ok(())
    }

    async fn stop(&self, name: &str) -> Result<()> {
        let output = Self::tart_cmd()
            .args(["stop", name])
            .output()
            .await
            .map_err(|e| crate::TachikomaError::Tart(format!("Failed to run tart stop: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::TachikomaError::Tart(format!(
                "tart stop failed: {stderr}"
            )));
        }

        Ok(())
    }

    async fn suspend(&self, name: &str) -> Result<()> {
        let output = Self::tart_cmd()
            .args(["suspend", name])
            .output()
            .await
            .map_err(|e| {
                crate::TachikomaError::Tart(format!("Failed to run tart suspend: {e}"))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::TachikomaError::Tart(format!(
                "tart suspend failed: {stderr}"
            )));
        }

        Ok(())
    }

    async fn delete(&self, name: &str) -> Result<()> {
        let output = Self::tart_cmd()
            .args(["delete", name])
            .output()
            .await
            .map_err(|e| {
                crate::TachikomaError::Tart(format!("Failed to run tart delete: {e}"))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::TachikomaError::Tart(format!(
                "tart delete failed: {stderr}"
            )));
        }

        Ok(())
    }

    async fn ip(&self, name: &str) -> Result<Option<IpAddr>> {
        let output = Self::tart_cmd()
            .args(["ip", name])
            .output()
            .await
            .map_err(|e| crate::TachikomaError::Tart(format!("Failed to run tart ip: {e}")))?;

        if !output.status.success() {
            return Ok(None);
        }

        let ip_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        match ip_str.parse::<IpAddr>() {
            Ok(ip) => Ok(Some(ip)),
            Err(_) => Ok(None),
        }
    }

    async fn exec(&self, name: &str, cmd: Vec<String>) -> Result<ExecOutput> {
        let mut args = vec!["exec".to_string(), name.to_string(), "--".to_string()];
        args.extend(cmd);

        let output = Self::tart_cmd()
            .args(&args)
            .output()
            .await
            .map_err(|e| crate::TachikomaError::Tart(format!("Failed to run tart exec: {e}")))?;

        Ok(ExecOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }
}
