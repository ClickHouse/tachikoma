use crate::Result;
use crate::state::{StateStore, VmStatus};
use crate::tart::TartRunner;

/// Suspend a VM. Currently always uses stop because tart suspend silently
/// breaks Linux VMs (exits 0 but creates an unresumable state).
/// TODO: Re-enable true suspend when targeting macOS VMs only.
pub async fn run(
    vm_name: &str,
    tart: &dyn TartRunner,
    state_store: &dyn StateStore,
) -> Result<VmStatus> {
    // Always use stop — tart suspend on Linux VMs exits 0 but creates
    // a broken "suspended" state that cannot be resumed.
    tart.stop(vm_name).await?;
    let status = VmStatus::Stopped;

    let mut state = state_store.load().await?;
    if let Some(entry) = state.find_vm_mut(vm_name) {
        entry.status = status;
    }
    state_store.save(&state).await?;

    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{MockStateStore, State, VmEntry};
    use crate::tart::MockTartRunner;
    use chrono::Utc;
    use std::path::PathBuf;

    fn test_vm_entry() -> VmEntry {
        VmEntry {
            name: "test-vm".to_string(),
            repo: "repo".to_string(),
            branch: "main".to_string(),
            worktree_path: PathBuf::from("/tmp/wt"),
            created_at: Utc::now(),
            last_used: Utc::now(),
            status: VmStatus::Running,
            ip: Some("192.168.64.10".to_string()),
        }
    }

    #[tokio::test]
    async fn test_suspend_uses_stop() {
        let mut tart = MockTartRunner::new();
        tart.expect_stop().returning(|_| Ok(()));

        let mut state = State::new();
        state.add_vm(test_vm_entry());

        let mut store = MockStateStore::new();
        store.expect_load().returning(move || Ok(state.clone()));
        store
            .expect_save()
            .withf(|s: &State| s.vms[0].status == VmStatus::Stopped)
            .returning(|_| Ok(()));

        let status = run("test-vm", &tart, &store).await.unwrap();
        assert_eq!(status, VmStatus::Stopped);
    }
}
