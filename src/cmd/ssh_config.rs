use std::path::PathBuf;

use crate::state::StateStore;
use crate::Result;

const MARKER_BEGIN: &str = "# BEGIN tachikoma managed";
const MARKER_END: &str = "# END tachikoma managed";

pub async fn install(state_store: &dyn StateStore, ssh_user: &str) -> Result<()> {
    let state = state_store.load().await?;
    let config_path = ssh_config_path()?;

    let mut existing = if config_path.exists() {
        tokio::fs::read_to_string(&config_path)
            .await
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Remove existing tachikoma block
    existing = remove_managed_block(&existing);

    // Build new block
    let mut block = format!("{MARKER_BEGIN}\n");
    for vm in &state.vms {
        if let Some(ip) = &vm.ip {
            block.push_str(&format!(
                "Host {}\n  HostName {}\n  User {}\n  StrictHostKeyChecking no\n  UserKnownHostsFile /dev/null\n  LogLevel ERROR\n\n",
                vm.name, ip, ssh_user
            ));
        }
    }
    block.push_str(MARKER_END);

    let new_contents = if existing.trim().is_empty() {
        block
    } else {
        format!("{}\n\n{}", existing.trim_end(), block)
    };

    if let Some(parent) = config_path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    tokio::fs::write(&config_path, new_contents)
        .await
        .map_err(|e| crate::TachikomaError::Ssh(format!("Failed to write SSH config: {e}")))?;

    Ok(())
}

pub async fn uninstall() -> Result<()> {
    let config_path = ssh_config_path()?;
    if !config_path.exists() {
        return Ok(());
    }

    let contents = tokio::fs::read_to_string(&config_path)
        .await
        .unwrap_or_default();
    let cleaned = remove_managed_block(&contents);
    tokio::fs::write(&config_path, cleaned)
        .await
        .map_err(|e| crate::TachikomaError::Ssh(format!("Failed to write SSH config: {e}")))?;

    Ok(())
}

pub async fn refresh(state_store: &dyn StateStore, ssh_user: &str) -> Result<()> {
    install(state_store, ssh_user).await
}

fn ssh_config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| {
        crate::TachikomaError::Ssh("Could not determine home directory".to_string())
    })?;
    Ok(home.join(".ssh").join("config"))
}

fn remove_managed_block(contents: &str) -> String {
    let mut result = String::new();
    let mut in_block = false;

    for line in contents.lines() {
        if line.trim() == MARKER_BEGIN {
            in_block = true;
            continue;
        }
        if line.trim() == MARKER_END {
            in_block = false;
            continue;
        }
        if !in_block {
            result.push_str(line);
            result.push('\n');
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_managed_block() {
        let input = "Host other\n  HostName 1.2.3.4\n\n# BEGIN tachikoma managed\nHost vm1\n  HostName 5.6.7.8\n# END tachikoma managed\n\nHost another\n  HostName 9.10.11.12\n";
        let result = remove_managed_block(input);
        assert!(!result.contains("tachikoma"));
        assert!(result.contains("Host other"));
        assert!(result.contains("Host another"));
    }

    #[test]
    fn test_remove_no_block() {
        let input = "Host other\n  HostName 1.2.3.4\n";
        let result = remove_managed_block(input);
        assert_eq!(result, "Host other\n  HostName 1.2.3.4\n");
    }
}
