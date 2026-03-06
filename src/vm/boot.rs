use std::net::IpAddr;
use std::time::Duration;

use crate::ssh::SshClient;
use crate::tart::TartRunner;
use crate::Result;

/// Boot detection configuration
#[derive(Debug, Clone)]
pub struct BootConfig {
    pub initial_delay: Duration,
    pub max_interval: Duration,
    pub backoff_factor: f64,
    pub timeout: Duration,
    pub ssh_user: String,
}

impl Default for BootConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(500),
            max_interval: Duration::from_secs(5),
            backoff_factor: 2.0,
            timeout: Duration::from_secs(120),
            ssh_user: "admin".to_string(),
        }
    }
}

/// Two-phase boot detection:
/// 1. Poll `tart ip` until we get an IP address
/// 2. Poll SSH connection until it succeeds
pub async fn wait_for_boot(
    tart: &dyn TartRunner,
    ssh: &dyn SshClient,
    vm_name: &str,
    config: &BootConfig,
) -> Result<IpAddr> {
    let start = tokio::time::Instant::now();
    let deadline = start + config.timeout;

    // Phase 1: Wait for IP
    let ip = poll_for_ip(tart, vm_name, config, deadline).await?;

    // Phase 2: Wait for SSH
    poll_for_ssh(ssh, ip, &config.ssh_user, config, deadline).await?;

    Ok(ip)
}

async fn poll_for_ip(
    tart: &dyn TartRunner,
    vm_name: &str,
    config: &BootConfig,
    deadline: tokio::time::Instant,
) -> Result<IpAddr> {
    let mut delay = config.initial_delay;

    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(crate::TachikomaError::Vm(format!(
                "Timed out waiting for VM '{vm_name}' to acquire an IP address"
            )));
        }

        tokio::time::sleep(delay).await;

        match tart.ip(vm_name).await {
            Ok(Some(ip)) => {
                tracing::debug!("VM '{vm_name}' acquired IP: {ip}");
                return Ok(ip);
            }
            Ok(None) => {
                tracing::trace!("VM '{vm_name}' has no IP yet, retrying...");
            }
            Err(e) => {
                tracing::trace!("Error polling IP for '{vm_name}': {e}");
            }
        }

        delay = Duration::from_secs_f64(delay.as_secs_f64() * config.backoff_factor)
            .min(config.max_interval);
    }
}

async fn poll_for_ssh(
    ssh: &dyn SshClient,
    ip: IpAddr,
    user: &str,
    config: &BootConfig,
    deadline: tokio::time::Instant,
) -> Result<()> {
    let mut delay = config.initial_delay;

    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(crate::TachikomaError::Vm(format!(
                "Timed out waiting for SSH on {ip}"
            )));
        }

        tokio::time::sleep(delay).await;

        match ssh.check_connection(ip, user).await {
            Ok(true) => {
                tracing::debug!("SSH connection to {ip} established");
                return Ok(());
            }
            Ok(false) => {
                tracing::trace!("SSH not ready on {ip}, retrying...");
            }
            Err(e) => {
                tracing::trace!("SSH check error on {ip}: {e}");
            }
        }

        delay = Duration::from_secs_f64(delay.as_secs_f64() * config.backoff_factor)
            .min(config.max_interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::MockSshClient;
    use crate::tart::MockTartRunner;
    use std::net::Ipv4Addr;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_immediate_boot() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));

        let mut mock_tart = MockTartRunner::new();
        mock_tart
            .expect_ip()
            .returning(move |_| Ok(Some(ip)));

        let mut mock_ssh = MockSshClient::new();
        mock_ssh
            .expect_check_connection()
            .returning(|_, _| Ok(true));

        let config = BootConfig {
            initial_delay: Duration::from_millis(10),
            timeout: Duration::from_secs(5),
            ..Default::default()
        };

        let result = wait_for_boot(&mock_tart, &mock_ssh, "test-vm", &config).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ip);
    }

    #[tokio::test]
    async fn test_ip_takes_a_few_polls() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));
        let call_count = AtomicU32::new(0);

        let mut mock_tart = MockTartRunner::new();
        mock_tart.expect_ip().returning(move |_| {
            let count = call_count.fetch_add(1, Ordering::SeqCst);
            if count < 2 {
                Ok(None)
            } else {
                Ok(Some(ip))
            }
        });

        let mut mock_ssh = MockSshClient::new();
        mock_ssh
            .expect_check_connection()
            .returning(|_, _| Ok(true));

        let config = BootConfig {
            initial_delay: Duration::from_millis(10),
            max_interval: Duration::from_millis(20),
            timeout: Duration::from_secs(5),
            ..Default::default()
        };

        let result = wait_for_boot(&mock_tart, &mock_ssh, "test-vm", &config).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_ssh_takes_a_few_polls() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));
        let ssh_count = AtomicU32::new(0);

        let mut mock_tart = MockTartRunner::new();
        mock_tart
            .expect_ip()
            .returning(move |_| Ok(Some(ip)));

        let mut mock_ssh = MockSshClient::new();
        mock_ssh
            .expect_check_connection()
            .returning(move |_, _| {
                let count = ssh_count.fetch_add(1, Ordering::SeqCst);
                Ok(count >= 2)
            });

        let config = BootConfig {
            initial_delay: Duration::from_millis(10),
            max_interval: Duration::from_millis(20),
            timeout: Duration::from_secs(5),
            ..Default::default()
        };

        let result = wait_for_boot(&mock_tart, &mock_ssh, "test-vm", &config).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_ip_timeout() {
        let mut mock_tart = MockTartRunner::new();
        mock_tart.expect_ip().returning(|_| Ok(None));

        let mock_ssh = MockSshClient::new();

        let config = BootConfig {
            initial_delay: Duration::from_millis(10),
            max_interval: Duration::from_millis(20),
            timeout: Duration::from_millis(100),
            ..Default::default()
        };

        let result = wait_for_boot(&mock_tart, &mock_ssh, "test-vm", &config).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Timed out"), "Expected timeout error, got: {err}");
    }

    #[tokio::test]
    async fn test_ssh_timeout() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));

        let mut mock_tart = MockTartRunner::new();
        mock_tart
            .expect_ip()
            .returning(move |_| Ok(Some(ip)));

        let mut mock_ssh = MockSshClient::new();
        mock_ssh
            .expect_check_connection()
            .returning(|_, _| Ok(false));

        let config = BootConfig {
            initial_delay: Duration::from_millis(10),
            max_interval: Duration::from_millis(20),
            timeout: Duration::from_millis(100),
            ..Default::default()
        };

        let result = wait_for_boot(&mock_tart, &mock_ssh, "test-vm", &config).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Timed out"), "Expected timeout error, got: {err}");
    }

    #[tokio::test]
    async fn test_backoff_increases() {
        // Verify that delays increase by checking total elapsed time is reasonable
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));
        let ip_count = AtomicU32::new(0);

        let mut mock_tart = MockTartRunner::new();
        mock_tart.expect_ip().returning(move |_| {
            let count = ip_count.fetch_add(1, Ordering::SeqCst);
            if count < 4 {
                Ok(None)
            } else {
                Ok(Some(ip))
            }
        });

        let mut mock_ssh = MockSshClient::new();
        mock_ssh
            .expect_check_connection()
            .returning(|_, _| Ok(true));

        let config = BootConfig {
            initial_delay: Duration::from_millis(10),
            max_interval: Duration::from_millis(100),
            backoff_factor: 2.0,
            timeout: Duration::from_secs(5),
            ssh_user: "admin".to_string(),
        };

        let start = tokio::time::Instant::now();
        let result = wait_for_boot(&mock_tart, &mock_ssh, "test-vm", &config).await;
        let elapsed = start.elapsed();

        assert!(result.is_ok());
        // With backoff: 10 + 20 + 40 + 80 = 150ms minimum
        assert!(
            elapsed >= Duration::from_millis(100),
            "Expected backoff delays, elapsed: {elapsed:?}"
        );
    }
}
