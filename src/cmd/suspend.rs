use crate::state::{StateStore, VmStatus};
use crate::tart::TartRunner;
use crate::Result;

pub async fn run(
    vm_name: &str,
    tart: &dyn TartRunner,
    state_store: &dyn StateStore,
) -> Result<()> {
    tart.suspend(vm_name).await?;

    let mut state = state_store.load().await?;
    if let Some(entry) = state.find_vm_mut(vm_name) {
        entry.status = VmStatus::Suspended;
    }
    state_store.save(&state).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{MockStateStore, State, VmEntry};
    use crate::tart::MockTartRunner;
    use chrono::Utc;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_suspend_vm() {
        let mut tart = MockTartRunner::new();
        tart.expect_suspend().returning(|_| Ok(()));

        let mut state = State::new();
        state.add_vm(VmEntry {
            name: "test-vm".to_string(),
            repo: "repo".to_string(),
            branch: "main".to_string(),
            worktree_path: PathBuf::from("/tmp/wt"),
            created_at: Utc::now(),
            last_used: Utc::now(),
            status: VmStatus::Running,
            ip: Some("192.168.64.10".to_string()),
        });

        let mut store = MockStateStore::new();
        store.expect_load().returning(move || Ok(state.clone()));
        store
            .expect_save()
            .withf(|s: &State| s.vms[0].status == VmStatus::Suspended)
            .returning(|_| Ok(()));

        let result = run("test-vm", &tart, &store).await;
        assert!(result.is_ok());
    }
}
