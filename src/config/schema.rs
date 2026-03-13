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
    pub sync_gh_auth: Option<bool>,
    /// Which ~/.claude subdirectories to share into VMs (default: rules, agents, plugins, skills)
    pub share_claude_dirs: Option<Vec<String>>,
    /// Whether to sync MCP server configs and their env vars into VMs (default: true)
    pub sync_mcp_servers: Option<bool>,
    /// Enable the built-in credential proxy (default: false)
    pub credential_proxy: Option<bool>,
    /// Port the credential proxy listens on (default: 19280)
    pub credential_proxy_port: Option<u16>,
    /// Address the credential proxy binds to (default: "192.168.64.1")
    pub credential_proxy_bind: Option<String>,
    /// Credential cache TTL in seconds (default: 300)
    pub credential_proxy_ttl_secs: Option<u64>,
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
            sync_gh_auth: other.sync_gh_auth.or(self.sync_gh_auth),
            share_claude_dirs: other.share_claude_dirs.or(self.share_claude_dirs),
            sync_mcp_servers: other.sync_mcp_servers.or(self.sync_mcp_servers),
            credential_proxy: other.credential_proxy.or(self.credential_proxy),
            credential_proxy_port: other.credential_proxy_port.or(self.credential_proxy_port),
            credential_proxy_bind: other.credential_proxy_bind.or(self.credential_proxy_bind),
            credential_proxy_ttl_secs: other
                .credential_proxy_ttl_secs
                .or(self.credential_proxy_ttl_secs),
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
    /// Sync host's gh CLI auth token into the VM (default: false)
    pub sync_gh_auth: bool,
    /// Which ~/.claude subdirectories to share into VMs (default: rules, agents, plugins, skills)
    pub share_claude_dirs: Vec<String>,
    /// Whether to sync MCP server configs and their env vars into VMs (default: true)
    pub sync_mcp_servers: bool,
    /// Enable the built-in credential proxy (default: false)
    pub credential_proxy: bool,
    /// Port the credential proxy listens on (default: 19280)
    pub credential_proxy_port: u16,
    /// Address the credential proxy binds to (default: "192.168.64.1")
    pub credential_proxy_bind: String,
    /// Credential cache TTL in seconds (default: 300)
    pub credential_proxy_ttl_secs: u64,
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
            sync_gh_auth: p.sync_gh_auth.unwrap_or(false),
            share_claude_dirs: {
                let dirs = p.share_claude_dirs.unwrap_or_else(|| {
                    crate::CLAUDE_SHARE_DIRS
                        .iter()
                        .map(|s| (*s).to_string())
                        .collect()
                });
                for d in &dirs {
                    if d.is_empty()
                        || d.contains('/')
                        || d.contains("..")
                        || !d
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                    {
                        return Err(crate::TachikomaError::Config(format!(
                            "Invalid share_claude_dirs entry '{d}': must contain only \
                             alphanumeric characters, hyphens, or underscores"
                        )));
                    }
                }
                dirs
            },
            sync_mcp_servers: p.sync_mcp_servers.unwrap_or(true),
            credential_proxy: p.credential_proxy.unwrap_or(true),
            credential_proxy_port: p.credential_proxy_port.unwrap_or(19280),
            credential_proxy_bind: {
                let bind = p
                    .credential_proxy_bind
                    .unwrap_or_else(|| "192.168.64.1".to_string());
                let addr: std::net::IpAddr = bind.parse().map_err(|_| {
                    crate::TachikomaError::Config(format!(
                        "Invalid credential_proxy_bind '{bind}': must be a valid IP address"
                    ))
                })?;
                if addr.is_unspecified() {
                    return Err(crate::TachikomaError::Config(
                        "credential_proxy_bind must not be 0.0.0.0 or [::] — \
                         the proxy would expose API keys to the network"
                            .to_string(),
                    ));
                }
                bind
            },
            credential_proxy_ttl_secs: p.credential_proxy_ttl_secs.unwrap_or(300),
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
        assert_eq!(
            config.share_claude_dirs,
            vec!["rules", "agents", "plugins", "skills"]
        );
        assert!(config.sync_mcp_servers);
        assert!(config.credential_proxy);
        assert_eq!(config.credential_proxy_port, 19280);
        assert_eq!(config.credential_proxy_bind, "192.168.64.1");
        assert_eq!(config.credential_proxy_ttl_secs, 300);
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
        assert!(defaults.share_claude_dirs.is_none());
        assert!(defaults.sync_mcp_servers.unwrap());
        assert!(defaults.credential_proxy.unwrap());
        assert_eq!(defaults.credential_proxy_port.unwrap(), 19280);
        assert_eq!(
            defaults.credential_proxy_bind.as_deref().unwrap(),
            "192.168.64.1"
        );
        assert_eq!(defaults.credential_proxy_ttl_secs.unwrap(), 300);
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
            sync_gh_auth: Some(true),
            share_claude_dirs: Some(vec!["rules".to_string(), "agents".to_string()]),
            sync_mcp_servers: Some(false),
            credential_proxy: Some(true),
            credential_proxy_port: Some(9000),
            credential_proxy_bind: Some("0.0.0.0".to_string()),
            credential_proxy_ttl_secs: Some(60),
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
        assert_eq!(deserialized.share_claude_dirs, original.share_claude_dirs);
        assert_eq!(deserialized.sync_mcp_servers, original.sync_mcp_servers);
        assert_eq!(deserialized.credential_proxy, original.credential_proxy);
        assert_eq!(
            deserialized.credential_proxy_port,
            original.credential_proxy_port
        );
        assert_eq!(
            deserialized.credential_proxy_bind,
            original.credential_proxy_bind
        );
        assert_eq!(
            deserialized.credential_proxy_ttl_secs,
            original.credential_proxy_ttl_secs
        );
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

    #[test]
    fn test_share_claude_dirs_valid_entries() {
        let config = Config::from_partial(PartialConfig {
            share_claude_dirs: Some(vec![
                "rules".to_string(),
                "my-agents".to_string(),
                "custom_dir".to_string(),
            ]),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            config.share_claude_dirs,
            vec!["rules", "my-agents", "custom_dir"]
        );
    }

    #[test]
    fn test_share_claude_dirs_rejects_path_traversal() {
        let result = Config::from_partial(PartialConfig {
            share_claude_dirs: Some(vec!["../../.ssh".to_string()]),
            ..Default::default()
        });
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Invalid share_claude_dirs entry"));
    }

    #[test]
    fn test_share_claude_dirs_rejects_shell_metacharacters() {
        for bad in &["rules;echo hi", "rules$(id)", "rules `id`", ""] {
            let result = Config::from_partial(PartialConfig {
                share_claude_dirs: Some(vec![bad.to_string()]),
                ..Default::default()
            });
            assert!(result.is_err(), "expected error for entry: {bad:?}");
        }
    }

    #[test]
    fn test_credential_proxy_bind_rejects_unspecified() {
        let result = Config::from_partial(PartialConfig {
            credential_proxy_bind: Some("0.0.0.0".to_string()),
            ..Default::default()
        });
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("0.0.0.0"),
            "Error should mention the bad value: {msg}"
        );
    }

    #[test]
    fn test_credential_proxy_bind_rejects_invalid_ip() {
        let result = Config::from_partial(PartialConfig {
            credential_proxy_bind: Some("not-an-ip".to_string()),
            ..Default::default()
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_credential_proxy_bind_accepts_valid_ip() {
        let config = Config::from_partial(PartialConfig {
            credential_proxy_bind: Some("10.0.0.1".to_string()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(config.credential_proxy_bind, "10.0.0.1");
    }

    #[test]
    fn test_share_claude_dirs_rejects_slash() {
        let result = Config::from_partial(PartialConfig {
            share_claude_dirs: Some(vec!["rules/subdir".to_string()]),
            ..Default::default()
        });
        assert!(result.is_err());
    }
}
