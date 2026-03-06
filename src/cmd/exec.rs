use std::net::IpAddr;

use crate::ssh::SshClient;
use crate::state::StateStore;
use crate::Result;

pub async fn run(
    vm_name: &str,
    cmd: &[String],
    ssh: &dyn SshClient,
    state_store: &dyn StateStore,
    ssh_user: &str,
) -> Result<String> {
    let state = state_store.load().await?;
    let entry = state.find_vm(vm_name).ok_or_else(|| {
        crate::TachikomaError::Vm(format!("VM '{vm_name}' not found"))
    })?;

    let ip_str = entry.ip.as_deref().ok_or_else(|| {
        crate::TachikomaError::Vm(format!("VM '{vm_name}' has no IP address"))
    })?;

    let ip: IpAddr = ip_str.parse().map_err(|e| {
        crate::TachikomaError::Vm(format!("Invalid IP address '{ip_str}': {e}"))
    })?;

    let full_cmd = cmd.join(" ");
    ssh.run_command(ip, ssh_user, &full_cmd).await
}
