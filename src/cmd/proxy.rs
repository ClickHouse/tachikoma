use crate::cli::ProxyAction;
use crate::config::Config;
use crate::proxy::{ProxyConfig, is_proxy_reachable};

/// Path to the proxy PID file.
fn pid_path() -> Option<std::path::PathBuf> {
    Some(crate::state::FileStateStore::default_path().join("proxy.pid"))
}

pub async fn run(action: Option<ProxyAction>, config: &Config) -> crate::Result<()> {
    match action {
        None
        | Some(ProxyAction::Start {
            port: None,
            bind: None,
            daemon: false,
        }) => {
            start_with_config(
                config.credential_proxy_port,
                &config.credential_proxy_bind,
                config,
                false,
            )
            .await
        }
        Some(ProxyAction::Start { port, bind, daemon }) => {
            let effective_port = port.unwrap_or(config.credential_proxy_port);
            let effective_bind = bind
                .as_deref()
                .unwrap_or(&config.credential_proxy_bind)
                .to_string();
            if daemon {
                daemonize(&effective_bind, effective_port)?;
                Ok(())
            } else {
                start_with_config(effective_port, &effective_bind, config, false).await
            }
        }
        Some(ProxyAction::Stop) => stop_proxy(),
        Some(ProxyAction::Status) => {
            status_proxy(&config.credential_proxy_bind, config.credential_proxy_port).await
        }
    }
}

/// Spawn a detached background copy of this process running `proxy start` (without --daemon)
/// and wait up to 2 s for it to become reachable.
fn daemonize(bind: &str, port: u16) -> crate::Result<()> {
    let exe = std::env::current_exe().map_err(|e| {
        crate::TachikomaError::Proxy(format!("Cannot resolve own executable path: {e}"))
    })?;

    #[cfg(unix)]
    {
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg("proxy")
            .arg("start")
            .arg("--port")
            .arg(port.to_string())
            .arg("--bind")
            .arg(bind)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        // Safety: setsid(2) is async-signal-safe.
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        cmd.spawn().map_err(|e| {
            crate::TachikomaError::Proxy(format!("Failed to spawn proxy daemon: {e}"))
        })?;
    }

    #[cfg(not(unix))]
    std::process::Command::new(&exe)
        .arg("proxy")
        .arg("start")
        .arg("--port")
        .arg(port.to_string())
        .arg("--bind")
        .arg(bind)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| crate::TachikomaError::Proxy(format!("Failed to spawn proxy daemon: {e}")))?;

    // Poll until reachable (up to 2 s)
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if std::net::TcpStream::connect(format!("{bind}:{port}")).is_ok() {
            println!("Credential proxy started in background at http://{bind}:{port}");
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            println!(
                "Proxy started but not yet reachable at {bind}:{port} — check with: tachikoma proxy status"
            );
            return Ok(());
        }
    }
}

async fn start_with_config(
    port: u16,
    bind: &str,
    config: &Config,
    _daemon: bool,
) -> crate::Result<()> {
    // Write our PID so `tachikoma proxy stop` can kill us
    write_pid()?;

    let proxy_config = ProxyConfig {
        bind: bind.to_string(),
        port,
        ttl_secs: config.credential_proxy_ttl_secs,
        credential_command: config.credential_command.clone(),
        api_key_command: config.api_key_command.clone(),
    };

    crate::proxy::start_proxy(proxy_config).await
}

fn write_pid() -> crate::Result<()> {
    let Some(path) = pid_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            crate::TachikomaError::Proxy(format!("Failed to create config dir: {e}"))
        })?;
    }
    let pid = std::process::id().to_string();
    std::fs::write(&path, &pid)
        .map_err(|e| crate::TachikomaError::Proxy(format!("Failed to write PID file: {e}")))?;
    Ok(())
}

fn stop_proxy() -> crate::Result<()> {
    let Some(path) = pid_path() else {
        println!("Proxy is not running (no PID file)");
        return Ok(());
    };

    let pid_str = std::fs::read_to_string(&path).map_err(|_| {
        crate::TachikomaError::Proxy("Proxy is not running (no PID file)".to_string())
    })?;

    let pid: i32 = pid_str.trim().parse().map_err(|_| {
        crate::TachikomaError::Proxy(format!("Invalid PID in file: '{}'", pid_str.trim()))
    })?;
    if pid <= 0 {
        return Err(crate::TachikomaError::Proxy(format!(
            "Invalid PID value: {pid}"
        )));
    }

    let result = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    if result != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) {
            let _ = std::fs::remove_file(&path);
            println!("Proxy process {pid} was not running (stale PID file removed)");
            return Ok(());
        }
        return Err(crate::TachikomaError::Proxy(format!(
            "Failed to stop proxy (PID {pid}): {err}"
        )));
    }

    let _ = std::fs::remove_file(&path);
    println!("Stopped credential proxy (PID {pid})");
    Ok(())
}

async fn status_proxy(bind: &str, port: u16) -> crate::Result<()> {
    let pid_info = pid_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| format!(" (PID {})", s.trim()))
        .unwrap_or_default();

    if is_proxy_reachable(bind, port).await {
        println!("Credential proxy is running at http://{bind}:{port}{pid_info}");
    } else {
        println!("Credential proxy is NOT running (nothing listening on {bind}:{port})");
    }
    Ok(())
}
