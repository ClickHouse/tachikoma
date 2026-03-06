use crate::cli::output::{print_table, OutputMode};
use crate::state::StateStore;
use crate::tart::TartRunner;
use crate::Result;

pub async fn run(
    repo_filter: Option<&str>,
    tart: &dyn TartRunner,
    state_store: &dyn StateStore,
    mode: OutputMode,
) -> Result<()> {
    let state = state_store.load().await?;
    let tart_vms = tart.list().await.unwrap_or_default();

    let vms: Vec<_> = if let Some(repo) = repo_filter {
        state.vms_for_repo(repo)
    } else {
        state.vms.iter().collect()
    };

    let rows: Vec<Vec<String>> = vms
        .iter()
        .map(|vm| {
            // Cross-reference with tart for live status
            let live_status = tart_vms
                .iter()
                .find(|t| t.name == vm.name)
                .map(|t| t.state.clone())
                .unwrap_or_else(|| format!("{:?}", vm.status));

            vec![
                vm.name.clone(),
                vm.repo.clone(),
                vm.branch.clone(),
                live_status,
                vm.ip.clone().unwrap_or_else(|| "-".to_string()),
                vm.last_used.format("%Y-%m-%d %H:%M").to_string(),
            ]
        })
        .collect();

    print_table(
        mode,
        &["NAME", "REPO", "BRANCH", "STATUS", "IP", "LAST USED"],
        &rows,
    );

    Ok(())
}
