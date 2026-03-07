use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Parsed output from `tart list --format json`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TartVmInfo {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "State")]
    pub state: String,
    #[serde(rename = "Disk", default)]
    pub disk: u64,
    #[serde(rename = "Size", default)]
    pub size: u64,
    #[serde(rename = "Source", default)]
    pub source: String,
    #[serde(rename = "Running", default)]
    pub running: bool,
    #[serde(rename = "Accessed", default)]
    pub accessed: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TartVmState {
    Running,
    Stopped,
    Suspended,
    Unknown,
}

impl From<&str> for TartVmState {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "running" => TartVmState::Running,
            "stopped" => TartVmState::Stopped,
            "suspended" => TartVmState::Suspended,
            _ => TartVmState::Unknown,
        }
    }
}

impl TartVmInfo {
    pub fn state_enum(&self) -> TartVmState {
        TartVmState::from(self.state.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct RunOpts {
    pub no_graphics: bool,
    pub dirs: Vec<DirMount>,
    pub rosetta: bool,
}

impl Default for RunOpts {
    fn default() -> Self {
        Self {
            no_graphics: true,
            dirs: vec![],
            rosetta: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DirMount {
    pub name: Option<String>,
    pub host_path: PathBuf,
    pub read_only: bool,
}

impl DirMount {
    /// Format as tart --dir argument: "name:path:ro", "name:path", "path:ro", or "path"
    pub fn to_tart_arg(&self) -> String {
        let mut parts = String::new();
        if let Some(ref name) = self.name {
            parts.push_str(name);
            parts.push(':');
        }
        parts.push_str(&self.host_path.display().to_string());
        if self.read_only {
            parts.push_str(":ro");
        }
        parts
    }
}

#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_tart_list() {
        let json = r#"[{"Name":"test-vm","State":"stopped","Disk":50,"Size":31,"Source":"local","Running":false,"Accessed":"2026-03-03T17:53:33Z"}]"#;
        let vms: Vec<TartVmInfo> = serde_json::from_str(json).unwrap();
        assert_eq!(vms.len(), 1);
        assert_eq!(vms[0].name, "test-vm");
        assert_eq!(vms[0].state, "stopped");
        assert_eq!(vms[0].disk, 50);
        assert_eq!(vms[0].size, 31);
        assert_eq!(vms[0].source, "local");
        assert!(!vms[0].running);
    }

    #[test]
    fn test_tart_vm_state_from_str() {
        assert_eq!(TartVmState::from("running"), TartVmState::Running);
        assert_eq!(TartVmState::from("stopped"), TartVmState::Stopped);
        assert_eq!(TartVmState::from("suspended"), TartVmState::Suspended);
        assert_eq!(TartVmState::from("unknown"), TartVmState::Unknown);
        assert_eq!(TartVmState::from("something_else"), TartVmState::Unknown);
    }

    #[test]
    fn test_state_enum() {
        let vm = TartVmInfo {
            name: "test-vm".to_string(),
            state: "Running".to_string(),
            disk: 50,
            size: 0,
            source: "local".to_string(),
            running: true,
            accessed: None,
        };
        assert_eq!(vm.state_enum(), TartVmState::Running);
    }

    #[test]
    fn test_run_opts_default() {
        let opts = RunOpts::default();
        assert!(opts.no_graphics);
        assert!(opts.dirs.is_empty());
        assert!(!opts.rosetta);
    }

    #[test]
    fn test_dir_mount_to_tart_arg() {
        let ro_mount = DirMount {
            name: None,
            host_path: PathBuf::from("/tmp/shared"),
            read_only: true,
        };
        assert_eq!(ro_mount.to_tart_arg(), "/tmp/shared:ro");

        let rw_mount = DirMount {
            name: None,
            host_path: PathBuf::from("/tmp/shared"),
            read_only: false,
        };
        assert_eq!(rw_mount.to_tart_arg(), "/tmp/shared");
    }

    #[test]
    fn test_dir_mount_named_to_tart_arg() {
        let named_ro = DirMount {
            name: Some("code".into()),
            host_path: PathBuf::from("/tmp/repo"),
            read_only: true,
        };
        assert_eq!(named_ro.to_tart_arg(), "code:/tmp/repo:ro");

        let named_rw = DirMount {
            name: Some("data".into()),
            host_path: PathBuf::from("/tmp/data"),
            read_only: false,
        };
        assert_eq!(named_rw.to_tart_arg(), "data:/tmp/data");
    }

    #[test]
    fn test_deserialize_minimal_fields() {
        let json = r#"[{"Name":"test-vm","State":"stopped"}]"#;
        let vms: Vec<TartVmInfo> = serde_json::from_str(json).unwrap();
        assert_eq!(vms.len(), 1);
        assert_eq!(vms[0].size, 0);
        assert_eq!(vms[0].disk, 0);
        assert!(!vms[0].running);
    }
}
