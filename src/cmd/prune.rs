use chrono::Utc;

use crate::Result;
use crate::state::StateStore;
use crate::tart::TartRunner;

pub struct PruneResult {
    pub pruned: Vec<String>,
    pub dry_run: bool,
}

pub async fn run(
    days: u64,
    dry_run: bool,
    tart: &dyn TartRunner,
    state_store: &dyn StateStore,
) -> Result<PruneResult> {
    let mut state = state_store.load().await?;
    let cutoff = Utc::now() - chrono::Duration::days(days as i64);

    let stale_names: Vec<String> = state
        .vms
        .iter()
        .filter(|vm| vm.last_used < cutoff)
        .map(|vm| vm.name.clone())
        .collect();

    if dry_run {
        return Ok(PruneResult {
            pruned: stale_names,
            dry_run: true,
        });
    }

    for name in &stale_names {
        let _ = tart.stop(name).await;
        let _ = tart.delete(name).await;
        state.remove_vm(name);
    }

    state_store.save(&state).await?;

    Ok(PruneResult {
        pruned: stale_names,
        dry_run: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{MockStateStore, State, VmEntry, VmStatus};
    use crate::tart::MockTartRunner;
    use std::path::PathBuf;

    fn old_entry(name: &str, days_ago: i64) -> VmEntry {
        VmEntry {
            name: name.to_string(),
            repo: "repo".to_string(),
            branch: "main".to_string(),
            worktree_path: PathBuf::from("/tmp/wt"),
            created_at: Utc::now() - chrono::Duration::days(days_ago + 1),
            last_used: Utc::now() - chrono::Duration::days(days_ago),
            status: VmStatus::Stopped,
            ip: None,
        }
    }

    #[tokio::test]
    async fn test_prune_old_vms() {
        let mut state = State::new();
        state.add_vm(old_entry("old-vm", 60));
        state.add_vm(old_entry("recent-vm", 5));

        let mut tart = MockTartRunner::new();
        tart.expect_stop().returning(|_| Ok(()));
        tart.expect_delete().returning(|_| Ok(()));

        let mut store = MockStateStore::new();
        store.expect_load().returning(move || Ok(state.clone()));
        store.expect_save().returning(|_| Ok(()));

        let result = run(30, false, &tart, &store).await.unwrap();
        assert_eq!(result.pruned.len(), 1);
        assert_eq!(result.pruned[0], "old-vm");
        assert!(!result.dry_run);
    }

    #[tokio::test]
    async fn test_prune_dry_run() {
        let mut state = State::new();
        state.add_vm(old_entry("old-vm", 60));

        let tart = MockTartRunner::new();

        let mut store = MockStateStore::new();
        store.expect_load().returning(move || Ok(state.clone()));

        let result = run(30, true, &tart, &store).await.unwrap();
        assert_eq!(result.pruned.len(), 1);
        assert!(result.dry_run);
    }

    #[tokio::test]
    async fn test_prune_nothing_stale() {
        let mut state = State::new();
        state.add_vm(old_entry("recent-vm", 5));

        let tart = MockTartRunner::new();

        let mut store = MockStateStore::new();
        store.expect_load().returning(move || Ok(state.clone()));
        store.expect_save().returning(|_| Ok(()));

        let result = run(30, false, &tart, &store).await.unwrap();
        assert!(result.pruned.is_empty());
    }
}
