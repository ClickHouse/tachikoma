pub mod schema;

pub use schema::{State, VmEntry, VmStatus};

use async_trait::async_trait;
use std::path::{Path, PathBuf};

use crate::Result;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait StateStore: Send + Sync {
    async fn load(&self) -> Result<State>;
    async fn save(&self, state: &State) -> Result<()>;
}

pub struct FileStateStore {
    state_path: PathBuf,
    lock_path: PathBuf,
}

impl FileStateStore {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            state_path: config_dir.join("state.json"),
            lock_path: config_dir.join("state.lock"),
        }
    }

    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("tachikoma")
    }

    fn with_lock<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T>,
    {
        use fd_lock::RwLock;

        // Ensure parent directory exists
        if let Some(parent) = self.lock_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                crate::TachikomaError::State(format!("Failed to create state directory: {e}"))
            })?;
        }

        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .read(true)
            .open(&self.lock_path)
            .map_err(|e| crate::TachikomaError::State(format!("Failed to open lock file: {e}")))?;

        let mut lock = RwLock::new(file);
        let _guard = lock.write().map_err(|e| {
            crate::TachikomaError::State(format!("Failed to acquire state lock: {e}"))
        })?;

        f()
    }
}

#[async_trait]
impl StateStore for FileStateStore {
    async fn load(&self) -> Result<State> {
        let state_path = self.state_path.clone();
        let lock_path = self.lock_path.clone();

        let store = FileStateStore {
            state_path,
            lock_path,
        };

        tokio::task::spawn_blocking(move || {
            store.with_lock(|| {
                if !store.state_path.exists() {
                    return Ok(State::new());
                }

                let contents = std::fs::read_to_string(&store.state_path).map_err(|e| {
                    crate::TachikomaError::State(format!("Failed to read state file: {e}"))
                })?;

                serde_json::from_str(&contents).map_err(|e| {
                    crate::TachikomaError::State(format!("Failed to parse state file: {e}"))
                })
            })
        })
        .await
        .map_err(|e| crate::TachikomaError::State(format!("Task join error: {e}")))?
    }

    async fn save(&self, state: &State) -> Result<()> {
        let state_path = self.state_path.clone();
        let lock_path = self.lock_path.clone();
        let state = state.clone();

        let store = FileStateStore {
            state_path,
            lock_path,
        };

        tokio::task::spawn_blocking(move || {
            store.with_lock(|| {
                // Ensure parent directory exists
                if let Some(parent) = store.state_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        crate::TachikomaError::State(format!(
                            "Failed to create state directory: {e}"
                        ))
                    })?;
                }

                // Atomic write: write to tmp file, then rename
                let tmp_path = store.state_path.with_extension("json.tmp");
                let contents = serde_json::to_string_pretty(&state).map_err(|e| {
                    crate::TachikomaError::State(format!("Failed to serialize state: {e}"))
                })?;

                std::fs::write(&tmp_path, contents).map_err(|e| {
                    crate::TachikomaError::State(format!("Failed to write state file: {e}"))
                })?;

                std::fs::rename(&tmp_path, &store.state_path).map_err(|e| {
                    crate::TachikomaError::State(format!("Failed to rename state file: {e}"))
                })?;

                Ok(())
            })
        })
        .await
        .map_err(|e| crate::TachikomaError::State(format!("Task join error: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;

    fn test_entry(name: &str) -> VmEntry {
        VmEntry {
            name: name.to_string(),
            repo: "test-repo".to_string(),
            branch: "main".to_string(),
            worktree_path: PathBuf::from("/tmp/test"),
            created_at: Utc::now(),
            last_used: Utc::now(),
            status: VmStatus::Stopped,
            ip: None,
        }
    }

    #[tokio::test]
    async fn test_load_empty_state() {
        let dir = TempDir::new().unwrap();
        let store = FileStateStore::new(dir.path());
        let state = store.load().await.unwrap();
        assert_eq!(state.version, 1);
        assert!(state.vms.is_empty());
    }

    #[tokio::test]
    async fn test_save_and_load() {
        let dir = TempDir::new().unwrap();
        let store = FileStateStore::new(dir.path());

        let mut state = State::new();
        state.add_vm(test_entry("test-vm"));

        store.save(&state).await.unwrap();

        let loaded = store.load().await.unwrap();
        assert_eq!(loaded.vms.len(), 1);
        assert_eq!(loaded.vms[0].name, "test-vm");
    }

    #[tokio::test]
    async fn test_save_atomic_creates_no_tmp() {
        let dir = TempDir::new().unwrap();
        let store = FileStateStore::new(dir.path());

        let state = State::new();
        store.save(&state).await.unwrap();

        // Tmp file should not remain
        assert!(!dir.path().join("state.json.tmp").exists());
        assert!(dir.path().join("state.json").exists());
    }

    #[tokio::test]
    async fn test_save_creates_directory() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("nested").join("dir");
        let store = FileStateStore::new(&nested);

        let state = State::new();
        store.save(&state).await.unwrap();

        assert!(nested.join("state.json").exists());
    }

    #[tokio::test]
    async fn test_multiple_saves() {
        let dir = TempDir::new().unwrap();
        let store = FileStateStore::new(dir.path());

        let mut state = State::new();
        state.add_vm(test_entry("vm-1"));
        store.save(&state).await.unwrap();

        state.add_vm(test_entry("vm-2"));
        store.save(&state).await.unwrap();

        let loaded = store.load().await.unwrap();
        assert_eq!(loaded.vms.len(), 2);
    }
}
