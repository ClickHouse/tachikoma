use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PartialConfig {
    pub base_image: Option<String>,
    pub vm_cpus: Option<u32>,
    pub vm_memory: Option<u32>,
    pub vm_display: Option<String>,
    pub ssh_user: Option<String>,
    pub ssh_port: Option<u16>,
    pub worktree_dir: Option<PathBuf>,
    pub provision_scripts: Option<Vec<String>>,
    pub claude_flags: Option<Vec<String>>,
    pub boot_timeout_secs: Option<u64>,
    pub prune_after_days: Option<u64>,
    pub credential_command: Option<String>,
    pub api_key_command: Option<String>,
}

impl PartialConfig {
    pub fn merge(self, other: PartialConfig) -> PartialConfig {
        PartialConfig {
            base_image: other.base_image.or(self.base_image),
            vm_cpus: other.vm_cpus.or(self.vm_cpus),
            vm_memory: other.vm_memory.or(self.vm_memory),
            vm_display: other.vm_display.or(self.vm_display),
            ssh_user: other.ssh_user.or(self.ssh_user),
            ssh_port: other.ssh_port.or(self.ssh_port),
            worktree_dir: other.worktree_dir.or(self.worktree_dir),
            provision_scripts: other.provision_scripts.or(self.provision_scripts),
            claude_flags: other.claude_flags.or(self.claude_flags),
            boot_timeout_secs: other.boot_timeout_secs.or(self.boot_timeout_secs),
            prune_after_days: other.prune_after_days.or(self.prune_after_days),
            credential_command: other.credential_command.or(self.credential_command),
            api_key_command: other.api_key_command.or(self.api_key_command),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub base_image: String,
    pub vm_cpus: u32,
    pub vm_memory: u32,
    pub vm_display: String,
    pub ssh_user: String,
    pub ssh_port: u16,
    pub worktree_dir: Option<PathBuf>,
    pub provision_scripts: Vec<String>,
    pub claude_flags: Vec<String>,
    pub boot_timeout_secs: u64,
    pub prune_after_days: u64,
    pub credential_command: Option<String>,
    pub api_key_command: Option<String>,
}

impl Config {
    pub fn from_partial(p: PartialConfig) -> crate::Result<Config> {
        Ok(Config {
            base_image: p.base_image.unwrap_or_else(|| "ubuntu".to_string()),
            vm_cpus: p.vm_cpus.unwrap_or(4),
            vm_memory: p.vm_memory.unwrap_or(8192),
            vm_display: p.vm_display.unwrap_or_else(|| "none".to_string()),
            ssh_user: p.ssh_user.unwrap_or_else(|| "admin".to_string()),
            ssh_port: p.ssh_port.unwrap_or(22),
            worktree_dir: p.worktree_dir,
            provision_scripts: p.provision_scripts.unwrap_or_default(),
            claude_flags: p.claude_flags.unwrap_or_default(),
            boot_timeout_secs: p.boot_timeout_secs.unwrap_or(120),
            prune_after_days: p.prune_after_days.unwrap_or(30),
            credential_command: p.credential_command,
            api_key_command: p.api_key_command,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_later_wins() {
        let first = PartialConfig {
            base_image: Some("old-image".to_string()),
            vm_cpus: Some(2),
            vm_memory: Some(4096),
            ..Default::default()
        };
        let second = PartialConfig {
            base_image: Some("new-image".to_string()),
            vm_cpus: Some(8),
            vm_memory: Some(16384),
            ..Default::default()
        };
        let merged = first.merge(second);
        assert_eq!(merged.base_image.unwrap(), "new-image");
        assert_eq!(merged.vm_cpus.unwrap(), 8);
        assert_eq!(merged.vm_memory.unwrap(), 16384);
    }

    #[test]
    fn test_merge_none_preserved() {
        let first = PartialConfig {
            base_image: Some("keep-me".to_string()),
            vm_cpus: Some(4),
            ssh_port: Some(2222),
            ..Default::default()
        };
        let second = PartialConfig::default();
        let merged = first.merge(second);
        assert_eq!(merged.base_image.unwrap(), "keep-me");
        assert_eq!(merged.vm_cpus.unwrap(), 4);
        assert_eq!(merged.ssh_port.unwrap(), 2222);
    }

    #[test]
    fn test_from_partial_all_defaults() {
        let config = Config::from_partial(PartialConfig::default()).unwrap();
        assert_eq!(config.base_image, "ubuntu");
        assert_eq!(config.vm_cpus, 4);
        assert_eq!(config.vm_memory, 8192);
        assert_eq!(config.vm_display, "none");
        assert_eq!(config.ssh_user, "admin");
        assert_eq!(config.ssh_port, 22);
        assert!(config.worktree_dir.is_none());
        assert!(config.provision_scripts.is_empty());
        assert!(config.claude_flags.is_empty());
        assert_eq!(config.boot_timeout_secs, 120);
        assert_eq!(config.prune_after_days, 30);
        assert!(config.credential_command.is_none());
        assert!(config.api_key_command.is_none());
    }

    #[test]
    fn test_from_partial_with_overrides() {
        let partial = PartialConfig {
            base_image: Some("custom-image".to_string()),
            vm_cpus: Some(16),
            ssh_port: Some(2222),
            credential_command: Some("op read secret".to_string()),
            ..Default::default()
        };
        let config = Config::from_partial(partial).unwrap();
        assert_eq!(config.base_image, "custom-image");
        assert_eq!(config.vm_cpus, 16);
        assert_eq!(config.ssh_port, 2222);
        assert_eq!(config.credential_command.unwrap(), "op read secret");
        assert_eq!(config.vm_memory, 8192);
    }

    #[test]
    fn test_default_config_values() {
        let defaults = super::super::defaults::default_config();
        assert_eq!(defaults.base_image.unwrap(), "ubuntu");
        assert_eq!(defaults.vm_cpus.unwrap(), 4);
        assert_eq!(defaults.vm_memory.unwrap(), 8192);
        assert_eq!(defaults.vm_display.unwrap(), "none");
        assert_eq!(defaults.ssh_user.unwrap(), "admin");
        assert_eq!(defaults.ssh_port.unwrap(), 22);
        assert!(defaults.worktree_dir.is_none());
        assert!(defaults.provision_scripts.unwrap().is_empty());
        assert!(defaults.claude_flags.unwrap().is_empty());
        assert_eq!(defaults.boot_timeout_secs.unwrap(), 120);
        assert_eq!(defaults.prune_after_days.unwrap(), 30);
        assert!(defaults.credential_command.is_none());
        assert!(defaults.api_key_command.is_none());
    }

    #[test]
    fn test_toml_roundtrip() {
        let original = PartialConfig {
            base_image: Some("test-image".to_string()),
            vm_cpus: Some(8),
            vm_memory: Some(16384),
            vm_display: Some("cocoa".to_string()),
            ssh_user: Some("testuser".to_string()),
            ssh_port: Some(2222),
            worktree_dir: Some(PathBuf::from("/tmp/worktrees")),
            provision_scripts: Some(vec!["setup.sh".to_string()]),
            claude_flags: Some(vec!["--verbose".to_string()]),
            boot_timeout_secs: Some(60),
            prune_after_days: Some(7),
            credential_command: Some("echo secret".to_string()),
            api_key_command: Some("echo key".to_string()),
        };
        let toml_str = toml::to_string(&original).unwrap();
        let deserialized: PartialConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(deserialized.base_image, original.base_image);
        assert_eq!(deserialized.vm_cpus, original.vm_cpus);
        assert_eq!(deserialized.vm_memory, original.vm_memory);
        assert_eq!(deserialized.ssh_port, original.ssh_port);
        assert_eq!(deserialized.worktree_dir, original.worktree_dir);
        assert_eq!(deserialized.provision_scripts, original.provision_scripts);
        assert_eq!(deserialized.claude_flags, original.claude_flags);
    }

    #[test]
    fn test_vm_cpus_default() {
        let config = Config::from_partial(PartialConfig::default()).unwrap();
        assert_eq!(config.vm_cpus, 4);
    }

    #[test]
    fn test_merge_chain() {
        let first = PartialConfig {
            base_image: Some("first".to_string()),
            vm_cpus: Some(2),
            vm_memory: Some(2048),
            ..Default::default()
        };
        let second = PartialConfig {
            base_image: Some("second".to_string()),
            vm_cpus: Some(4),
            ..Default::default()
        };
        let third = PartialConfig {
            base_image: Some("third".to_string()),
            ..Default::default()
        };
        let merged = first.merge(second).merge(third);
        assert_eq!(merged.base_image.unwrap(), "third");
        assert_eq!(merged.vm_cpus.unwrap(), 4);
        assert_eq!(merged.vm_memory.unwrap(), 2048);
    }
}
