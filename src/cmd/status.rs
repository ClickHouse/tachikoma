use crate::cli::output::{print_success, OutputMode};
use crate::state::StateStore;
use crate::tart::TartRunner;
use crate::Result;

pub async fn run(
    vm_name: &str,
    tart: &dyn TartRunner,
    state_store: &dyn StateStore,
    mode: OutputMode,
) -> Result<()> {
    let state = state_store.load().await?;
    let tart_vms = tart.list().await.unwrap_or_default();

    let entry = state.find_vm(vm_name).ok_or_else(|| {
        crate::TachikomaError::Vm(format!("VM '{vm_name}' not found in state"))
    })?;

    let live_status = tart_vms
        .iter()
        .find(|t| t.name == vm_name)
        .map(|t| t.state.clone())
        .unwrap_or_else(|| format!("{:?}", entry.status));

    let data = serde_json::json!({
        "name": entry.name,
        "repo": entry.repo,
        "branch": entry.branch,
        "status": live_status,
        "ip": entry.ip,
        "worktree": entry.worktree_path,
        "created": entry.created_at.to_rfc3339(),
        "last_used": entry.last_used.to_rfc3339(),
    });

    match mode {
        OutputMode::Json => {
            print_success(mode, "status", Some(data));
        }
        _ => {
            println!("Name:      {}", entry.name);
            println!("Repo:      {}", entry.repo);
            println!("Branch:    {}", entry.branch);
            println!("Status:    {}", live_status);
            println!(
                "IP:        {}",
                entry.ip.as_deref().unwrap_or("-")
            );
            println!("Worktree:  {}", entry.worktree_path.display());
            println!("Created:   {}", entry.created_at.format("%Y-%m-%d %H:%M"));
            println!("Last used: {}", entry.last_used.format("%Y-%m-%d %H:%M"));
        }
    }

    Ok(())
}
