use crate::ssh::SshClient;
use crate::tart::{RunOpts, TartRunner};
use crate::vm::boot::{wait_for_boot, BootConfig};
use crate::Result;

pub const DEFAULT_GOLDEN_IMAGE: &str = "tachikoma-golden";
const BUILDER_VM: &str = "tachikoma-builder";

pub async fn pull(image_name: &str, tart: &dyn TartRunner) -> Result<()> {
    tracing::info!("Pulling image '{image_name}'...");
    tart.clone_vm(image_name, &format!("{image_name}-local"))
        .await
}

pub async fn list(tart: &dyn TartRunner) -> Result<Vec<String>> {
    let vms = tart.list().await?;
    Ok(vms.into_iter().map(|vm| vm.name).collect())
}

/// Build a golden image: clone base → boot headless → install Claude → stop → clone to output.
///
/// The builder VM (`tachikoma-builder`) is always cleaned up, even on failure.
pub async fn build(
    base_image: &str,
    output_name: &str,
    force: bool,
    tart: &dyn TartRunner,
    ssh: &dyn SshClient,
    on_status: &dyn Fn(&str),
) -> Result<()> {
    // Guard: refuse to clobber an existing golden image unless --force
    if !force {
        let existing = tart.list().await?;
        if existing.iter().any(|vm| vm.name == output_name) {
            return Err(crate::TachikomaError::Vm(format!(
                "Image '{output_name}' already exists. Use --force to overwrite."
            )));
        }
    }

    // Ensure no stale builder VM from a previous failed run
    let existing = tart.list().await?;
    if existing.iter().any(|vm| vm.name == BUILDER_VM) {
        on_status("Cleaning up stale builder VM...");
        tart.delete(BUILDER_VM).await.ok();
    }

    on_status(&format!(
        "Cloning base image '{base_image}' → {BUILDER_VM}..."
    ));
    tart.clone_vm(base_image, BUILDER_VM)
        .await
        .map_err(|e| crate::TachikomaError::Vm(format!("Failed to clone base image: {e}")))?;

    // Boot headless — no git mounts needed for image build
    on_status("Booting builder VM...");
    let run_opts = RunOpts {
        no_graphics: true,
        dirs: vec![],
        rosetta: false,
    };
    tart.run(BUILDER_VM, &run_opts)
        .await
        .map_err(|e| crate::TachikomaError::Vm(format!("Failed to start builder VM: {e}")))?;

    // Wait for boot (SSH port open)
    let boot_cfg = BootConfig::default();
    on_status("Waiting for builder VM to boot...");
    let result = wait_for_boot(tart, ssh, BUILDER_VM, &boot_cfg).await;
    if let Err(e) = result {
        cleanup(tart, BUILDER_VM, output_name, force).await;
        return Err(e);
    }

    // Install Claude
    on_status("Installing Claude Code in builder VM...");
    if let Err(e) = crate::provision::install_claude(tart, BUILDER_VM).await {
        cleanup(tart, BUILDER_VM, output_name, force).await;
        return Err(e);
    }

    // Stop the builder cleanly before cloning
    on_status("Stopping builder VM...");
    if let Err(e) = tart.stop(BUILDER_VM).await {
        cleanup(tart, BUILDER_VM, output_name, force).await;
        return Err(crate::TachikomaError::Vm(format!(
            "Failed to stop builder VM: {e}"
        )));
    }

    // If --force, delete the existing golden image first
    if force {
        let existing = tart.list().await?;
        if existing.iter().any(|vm| vm.name == output_name) {
            on_status(&format!("Deleting existing '{output_name}'..."));
            tart.delete(output_name).await.ok();
        }
    }

    // Clone builder → golden image
    on_status(&format!("Cloning builder VM → '{output_name}'..."));
    if let Err(e) = tart.clone_vm(BUILDER_VM, output_name).await {
        tart.delete(BUILDER_VM).await.ok();
        return Err(crate::TachikomaError::Vm(format!(
            "Failed to clone builder to golden image: {e}"
        )));
    }

    // Delete the builder VM
    on_status("Cleaning up builder VM...");
    tart.delete(BUILDER_VM).await.ok();

    tracing::info!("Golden image '{output_name}' built successfully");
    Ok(())
}

/// Best-effort cleanup after a failed build: stop + delete builder, optionally delete partial golden.
async fn cleanup(tart: &dyn TartRunner, builder: &str, golden: &str, force: bool) {
    tart.stop(builder).await.ok();
    tart.delete(builder).await.ok();
    // Only delete the golden image if --force was specified (we created it in that case)
    if force {
        tart.delete(golden).await.ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::MockSshClient;
    use crate::tart::types::{ExecOutput, TartVmInfo};
    use crate::tart::MockTartRunner;
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::{Arc, Mutex};

    fn vm_info(name: &str) -> TartVmInfo {
        TartVmInfo {
            name: name.to_string(),
            state: "stopped".to_string(),
            disk: 0,
            size: 0,
            source: String::new(),
            running: false,
            accessed: None,
        }
    }

    fn mock_tart_for_build(ip: IpAddr) -> MockTartRunner {
        let mut tart = MockTartRunner::new();
        tart.expect_list().returning(|| Ok(vec![]));
        tart.expect_clone_vm().returning(|_, _| Ok(()));
        tart.expect_run().returning(|_, _| Ok(()));
        tart.expect_ip_wait().returning(move |_, _| Ok(Some(ip)));
        tart.expect_exec().returning(|_, _| {
            Ok(ExecOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            })
        });
        tart.expect_stop().returning(|_| Ok(()));
        tart.expect_delete().returning(|_| Ok(()));
        tart
    }

    #[tokio::test]
    async fn test_build_happy_path() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));
        let tart = mock_tart_for_build(ip);

        let mut ssh = MockSshClient::new();
        ssh.expect_check_port_open().returning(|_| Ok(true));

        let result = build("ubuntu", DEFAULT_GOLDEN_IMAGE, false, &tart, &ssh, &|_| {}).await;
        assert!(result.is_ok(), "build failed: {result:?}");
    }

    #[tokio::test]
    async fn test_build_refuses_existing_without_force() {
        let mut tart = MockTartRunner::new();
        tart.expect_list()
            .returning(|| Ok(vec![vm_info(DEFAULT_GOLDEN_IMAGE)]));

        let ssh = MockSshClient::new();
        let result = build("ubuntu", DEFAULT_GOLDEN_IMAGE, false, &tart, &ssh, &|_| {}).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("already exists"),
            "Expected 'already exists', got: {msg}"
        );
    }

    #[tokio::test]
    async fn test_build_with_force_overwrites() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 64, 10));
        let mut tart = MockTartRunner::new();

        // list calls: (1) stale builder check → empty, (2) force-delete check → golden exists
        let call_count = Arc::new(Mutex::new(0u32));
        let cc = call_count.clone();
        tart.expect_list().returning(move || {
            let mut n = cc.lock().unwrap();
            *n += 1;
            // On 2nd call (after stop, before clone), golden exists so --force deletes it
            if *n == 2 {
                Ok(vec![vm_info(DEFAULT_GOLDEN_IMAGE)])
            } else {
                Ok(vec![])
            }
        });
        tart.expect_clone_vm().returning(|_, _| Ok(()));
        tart.expect_run().returning(|_, _| Ok(()));
        tart.expect_ip_wait().returning(move |_, _| Ok(Some(ip)));
        tart.expect_exec().returning(|_, _| {
            Ok(ExecOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            })
        });
        tart.expect_stop().returning(|_| Ok(()));
        tart.expect_delete().returning(|_| Ok(()));

        let mut ssh = MockSshClient::new();
        ssh.expect_check_port_open().returning(|_| Ok(true));

        let result = build("ubuntu", DEFAULT_GOLDEN_IMAGE, true, &tart, &ssh, &|_| {}).await;
        assert!(result.is_ok(), "build with --force failed: {result:?}");
    }

    #[tokio::test]
    async fn test_list() {
        let mut tart = MockTartRunner::new();
        tart.expect_list().returning(|| {
            Ok(vec![
                vm_info("tachikoma-golden"),
                vm_info("tachikoma-myapp-main"),
            ])
        });
        let images = list(&tart).await.unwrap();
        assert_eq!(images, vec!["tachikoma-golden", "tachikoma-myapp-main"]);
    }
}
