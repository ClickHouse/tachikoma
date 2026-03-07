use crate::ssh::SshClient;
use crate::state::StateStore;
use crate::Result;

pub async fn run(
    vm_name: &str,
    ssh: &dyn SshClient,
    state_store: &dyn StateStore,
    ssh_user: &str,
) -> Result<()> {
    let state = state_store.load().await?;
    let entry = state
        .find_vm(vm_name)
        .ok_or_else(|| crate::TachikomaError::Vm(format!("VM '{vm_name}' not found")))?;

    let ip = entry.parsed_ip()?;
    ssh.connect_interactive(ip, ssh_user)
}
