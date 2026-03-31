use crate::Result;
use crate::state::StateStore;
use crate::tart::TartRunner;

pub async fn run(vm_name: &str, tart: &dyn TartRunner, state_store: &dyn StateStore) -> Result<()> {
    // Stop first if running (ignore errors)
    let _ = tart.stop(vm_name).await;

    // Delete the VM
    tart.delete(vm_name).await?;

    // Remove from state
    let mut state = state_store.load().await?;
    state.remove_vm(vm_name);
    state_store.save(&state).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{MockStateStore, State, VmEntry, VmStatus};
    use crate::tart::MockTartRunner;
    use chrono::Utc;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_destroy_vm() {
        let mut tart = MockTartRunner::new();
        tart.expect_stop().returning(|_| Ok(()));
        tart.expect_delete().returning(|_| Ok(()));

        let mut state = State::new();
        state.add_vm(VmEntry {
            name: "test-vm".to_string(),
            repo: "repo".to_string(),
            branch: "main".to_string(),
            worktree_path: PathBuf::from("/tmp/wt"),
            created_at: Utc::now(),
            last_used: Utc::now(),
            status: VmStatus::Running,
            ip: None,
        });

        let mut store = MockStateStore::new();
        store.expect_load().returning(move || Ok(state.clone()));
        store
            .expect_save()
            .withf(|s: &State| s.vms.is_empty())
            .returning(|_| Ok(()));

        let result = run("test-vm", &tart, &store).await;
        assert!(result.is_ok());
    }
}
