use crate::Result;
use crate::cli::output::OutputMode;

#[derive(Debug)]
pub struct CheckResult {
    pub name: String,
    pub passed: bool,
    pub message: String,
}

pub async fn run(mode: OutputMode) -> Result<Vec<CheckResult>> {
    let mut results = Vec::new();

    results.push(check_tart().await);
    results.push(check_git().await);
    results.push(check_ssh().await);

    match mode {
        OutputMode::Json => {
            let data: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "check": r.name,
                        "passed": r.passed,
                        "message": r.message,
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&data).unwrap_or_default()
            );
        }
        _ => {
            for r in &results {
                let icon = if r.passed { "ok" } else { "FAIL" };
                println!("[{icon}] {}: {}", r.name, r.message);
            }
        }
    }

    Ok(results)
}

async fn check_tart() -> CheckResult {
    match tokio::process::Command::new("tart")
        .arg("--version")
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            CheckResult {
                name: "tart".to_string(),
                passed: true,
                message: format!("Found: {version}"),
            }
        }
        _ => CheckResult {
            name: "tart".to_string(),
            passed: false,
            message: "Not found. Install with: brew install cirruslabs/cli/tart".to_string(),
        },
    }
}

async fn check_git() -> CheckResult {
    match tokio::process::Command::new("git")
        .arg("--version")
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            CheckResult {
                name: "git".to_string(),
                passed: true,
                message: format!("Found: {version}"),
            }
        }
        _ => CheckResult {
            name: "git".to_string(),
            passed: false,
            message: "Not found".to_string(),
        },
    }
}

async fn check_ssh() -> CheckResult {
    match tokio::process::Command::new("ssh").arg("-V").output().await {
        Ok(output) => {
            // ssh -V prints to stderr
            let version = String::from_utf8_lossy(&output.stderr).trim().to_string();
            CheckResult {
                name: "ssh".to_string(),
                passed: true,
                message: format!("Found: {version}"),
            }
        }
        _ => CheckResult {
            name: "ssh".to_string(),
            passed: false,
            message: "Not found".to_string(),
        },
    }
}
