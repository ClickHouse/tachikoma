use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum VmStatus {
    Running,
    Suspended,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmEntry {
    pub name: String,
    pub repo: String,
    pub branch: String,
    pub worktree_path: PathBuf,
    pub created_at: DateTime<Utc>,
    pub last_used: DateTime<Utc>,
    pub status: VmStatus,
    pub ip: Option<String>,
}

impl VmEntry {
    /// Parse the stored IP string into a typed `IpAddr`, returning a
    /// descriptive error when the VM has no IP or the value is malformed.
    pub fn parsed_ip(&self) -> crate::Result<std::net::IpAddr> {
        let ip_str = self.ip.as_deref().ok_or_else(|| {
            crate::TachikomaError::Vm(format!("VM '{}' has no IP address", self.name))
        })?;

        ip_str
            .parse()
            .map_err(|e| crate::TachikomaError::Vm(format!("Invalid IP address '{ip_str}': {e}")))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct State {
    pub version: u32,
    pub vms: Vec<VmEntry>,
}

impl State {
    pub fn new() -> Self {
        Self {
            version: 1,
            vms: Vec::new(),
        }
    }

    pub fn find_vm(&self, name: &str) -> Option<&VmEntry> {
        self.vms.iter().find(|vm| vm.name == name)
    }

    pub fn find_vm_mut(&mut self, name: &str) -> Option<&mut VmEntry> {
        self.vms.iter_mut().find(|vm| vm.name == name)
    }

    pub fn add_vm(&mut self, entry: VmEntry) {
        self.vms.push(entry);
    }

    pub fn remove_vm(&mut self, name: &str) -> Option<VmEntry> {
        let index = self.vms.iter().position(|vm| vm.name == name)?;
        Some(self.vms.remove(index))
    }

    pub fn vms_for_repo(&self, repo: &str) -> Vec<&VmEntry> {
        self.vms.iter().filter(|vm| vm.repo == repo).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_entry(name: &str, repo: &str, branch: &str) -> VmEntry {
        VmEntry {
            name: name.to_string(),
            repo: repo.to_string(),
            branch: branch.to_string(),
            worktree_path: PathBuf::from(format!("/tmp/{}", name)),
            created_at: Utc::now(),
            last_used: Utc::now(),
            status: VmStatus::Stopped,
            ip: None,
        }
    }

    #[test]
    fn test_new_state() {
        let state = State::new();
        assert_eq!(state.version, 1);
        assert!(state.vms.is_empty());
    }

    #[test]
    fn test_add_and_find_vm() {
        let mut state = State::new();
        state.add_vm(test_entry("test-vm", "my-repo", "main"));

        let found = state.find_vm("test-vm");
        assert!(found.is_some());
        assert_eq!(found.unwrap().repo, "my-repo");
    }

    #[test]
    fn test_find_vm_not_found() {
        let state = State::new();
        assert!(state.find_vm("nonexistent").is_none());
    }

    #[test]
    fn test_remove_vm() {
        let mut state = State::new();
        state.add_vm(test_entry("removable", "repo", "main"));

        let removed = state.remove_vm("removable");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().name, "removable");
        assert!(state.find_vm("removable").is_none());
    }

    #[test]
    fn test_vms_for_repo() {
        let mut state = State::new();
        state.add_vm(test_entry("vm1", "repo-a", "main"));
        state.add_vm(test_entry("vm2", "repo-b", "main"));
        state.add_vm(test_entry("vm3", "repo-a", "feat"));

        let repo_a_vms = state.vms_for_repo("repo-a");
        assert_eq!(repo_a_vms.len(), 2);
        assert!(repo_a_vms.iter().all(|vm| vm.repo == "repo-a"));

        let repo_b_vms = state.vms_for_repo("repo-b");
        assert_eq!(repo_b_vms.len(), 1);
    }

    #[test]
    fn test_json_roundtrip() {
        let mut state = State::new();
        state.add_vm(test_entry("roundtrip-vm", "my-repo", "main"));

        let json = serde_json::to_string(&state).expect("serialize");
        let deserialized: State = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.version, state.version);
        assert_eq!(deserialized.vms.len(), 1);
        assert_eq!(deserialized.vms[0].name, "roundtrip-vm");
        assert_eq!(deserialized.vms[0].repo, "my-repo");
    }

    #[test]
    fn test_vm_status_serialization() {
        let running = serde_json::to_string(&VmStatus::Running).expect("serialize");
        assert_eq!(running, "\"running\"");

        let suspended = serde_json::to_string(&VmStatus::Suspended).expect("serialize");
        assert_eq!(suspended, "\"suspended\"");

        let stopped = serde_json::to_string(&VmStatus::Stopped).expect("serialize");
        assert_eq!(stopped, "\"stopped\"");
    }
}
