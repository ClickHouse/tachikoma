use super::schema::PartialConfig;

pub fn default_config() -> PartialConfig {
    PartialConfig {
        base_image: Some("ubuntu".to_string()),
        vm_cpus: Some(4),
        vm_memory: Some(8192),
        vm_display: Some("none".to_string()),
        ssh_user: Some("admin".to_string()),
        ssh_port: Some(22),
        worktree_dir: None,
        provision_scripts: Some(vec![]),
        claude_flags: Some(vec![]),
        boot_timeout_secs: Some(120),
        prune_after_days: Some(30),
        credential_command: None,
        api_key_command: None,
        sync_gh_auth: Some(false),
        share_claude_dirs: None,
        sync_mcp_servers: Some(true),
        credential_proxy: Some(true),
        credential_proxy_port: Some(19280),
        credential_proxy_bind: Some("192.168.64.1".to_string()),
        credential_proxy_ttl_secs: Some(300),
    }
}
