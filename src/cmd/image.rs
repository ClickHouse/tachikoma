use crate::tart::TartRunner;
use crate::Result;

pub async fn pull(image_name: &str, tart: &dyn TartRunner) -> Result<()> {
    tracing::info!("Pulling image '{image_name}'...");
    tart.clone_vm(image_name, &format!("{image_name}-local"))
        .await
}

pub async fn list(tart: &dyn TartRunner) -> Result<Vec<String>> {
    let vms = tart.list().await?;
    Ok(vms.into_iter().map(|vm| vm.name).collect())
}
