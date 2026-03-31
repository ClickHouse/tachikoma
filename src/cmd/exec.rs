use crate::Result;
use crate::ssh::SshClient;
use crate::state::StateStore;

pub async fn run(
    vm_name: &str,
    cmd: &[String],
    ssh: &dyn SshClient,
    state_store: &dyn StateStore,
    ssh_user: &str,
) -> Result<String> {
    let state = state_store.load().await?;
    let entry = state
        .find_vm(vm_name)
        .ok_or_else(|| crate::TachikomaError::Vm(format!("VM '{vm_name}' not found")))?;

    let ip = entry.parsed_ip()?;
    let full_cmd = cmd.join(" ");
    ssh.run_command(ip, ssh_user, &full_cmd).await
}
