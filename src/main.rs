use clap::{CommandFactory, Parser};
use indicatif::{ProgressBar, ProgressStyle};
use tachikoma::cli::output::{print_error, print_success, OutputMode};
use tachikoma::cli::{Cli, Command, ImageAction, SshAction};
use tachikoma::config::{ConfigLoader, FileConfigLoader};
use tachikoma::ssh::RealSshClient;
use tachikoma::state::{FileStateStore, StateStore};
use tachikoma::tart::RealTartRunner;
use tachikoma::worktree::{GitWorktree, RealGitWorktree};

#[tokio::main]
async fn main() {
    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let mode = OutputMode::from_flags(cli.json, cli.verbose);

    if let Err(e) = run(cli, mode).await {
        print_error(mode, &e.to_string());
        std::process::exit(1);
    }
}

/// Returns true for commands that require tart to be installed.
fn command_needs_tart(cmd: &Option<Command>) -> bool {
    !matches!(
        cmd,
        Some(Command::Doctor)
            | Some(Command::Completions { .. })
            | Some(Command::Mcp)
            | Some(Command::Config { .. })
    )
}

/// Check whether `tart` is available on PATH. Runs `tart --version` as a fast probe.
async fn check_tart_available() -> bool {
    tokio::process::Command::new("tart")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn run(cli: Cli, mode: OutputMode) -> tachikoma::Result<()> {
    if command_needs_tart(&cli.command) && !check_tart_available().await {
        eprintln!("error: tart is not installed. Install it with:");
        eprintln!("  brew install cirruslabs/cli/tart");
        eprintln!();
        eprintln!("Run 'tachikoma doctor' for a full system check.");
        std::process::exit(1);
    }

    let tart = RealTartRunner::new();
    let ssh = RealSshClient::new();
    let git = RealGitWorktree::new();
    let state_dir = FileStateStore::default_path();
    let state_store = FileStateStore::new(&state_dir);
    let config_loader = FileConfigLoader::new();
    let cwd = std::env::current_dir().map_err(|e| {
        tachikoma::TachikomaError::Other(format!("Failed to get current directory: {e}"))
    })?;

    // Resolve repo root for config loading (best effort)
    let repo_root = git.find_repo_root(&cwd).await.ok();

    let config = config_loader.load(repo_root.clone()).await?;

    match cli.command {
        // No subcommand: zero-arg spawn (or branch shorthand)
        None => {
            let branch = cli.branch.as_deref();
            let spinner = make_spinner(mode);
            let result = tachikoma::cmd::spawn::run(
                branch,
                &cwd,
                &tart,
                &ssh,
                &git,
                &state_store,
                &config,
                true, // interactive
                &|msg: &str| spinner.set_message(msg.to_owned()),
            )
            .await;
            spinner.finish_and_clear();
            let result = result?;

            match &result {
                tachikoma::vm::SpawnResult::Reconnected { name, ip } => {
                    print_success(mode, &format!("Reconnected to {name} ({ip})"), None);
                }
                tachikoma::vm::SpawnResult::Resumed { name, ip } => {
                    print_success(mode, &format!("Resumed {name} ({ip})"), None);
                }
                tachikoma::vm::SpawnResult::Started { name, ip } => {
                    print_success(mode, &format!("Started {name} ({ip})"), None);
                }
                tachikoma::vm::SpawnResult::Created { name, ip } => {
                    print_success(mode, &format!("Created {name} ({ip})"), None);
                }
            }
        }

        Some(Command::Spawn { branch }) => {
            let spinner = make_spinner(mode);
            let result = tachikoma::cmd::spawn::run(
                branch.as_deref(),
                &cwd,
                &tart,
                &ssh,
                &git,
                &state_store,
                &config,
                true,
                &|msg: &str| spinner.set_message(msg.to_owned()),
            )
            .await;
            spinner.finish_and_clear();
            let result = result?;
            print_success(
                mode,
                &format!("VM {} ready at {}", result.name(), result.ip()),
                None,
            );
        }

        Some(Command::Enter) => {
            let branch = git.current_branch(&cwd).await?;
            let (repo_name, _) = resolve_repo(&git, &cwd).await?;
            let vm_name = tachikoma::vm_name(&repo_name, &branch);
            tachikoma::cmd::enter::run(&vm_name, &ssh, &state_store, &config.ssh_user).await?;
        }

        Some(Command::Exec { cmd }) => {
            let branch = git.current_branch(&cwd).await?;
            let (repo_name, _) = resolve_repo(&git, &cwd).await?;
            let vm_name = tachikoma::vm_name(&repo_name, &branch);
            let output =
                tachikoma::cmd::exec::run(&vm_name, &cmd, &ssh, &state_store, &config.ssh_user)
                    .await?;
            print!("{output}");
        }

        Some(Command::Halt { name }) => {
            let vm_name = resolve_vm_name(name, &git, &cwd).await?;
            tachikoma::cmd::halt::run(&vm_name, &tart, &state_store).await?;
            print_success(mode, &format!("Stopped {vm_name}"), None);
        }

        Some(Command::Suspend { name }) => {
            let vm_name = resolve_vm_name(name, &git, &cwd).await?;
            tachikoma::cmd::suspend::run(&vm_name, &tart, &state_store).await?;
            print_success(mode, &format!("Stopped {vm_name}"), None);
        }

        Some(Command::Destroy { name, force }) => {
            let vm_name = resolve_vm_name(name, &git, &cwd).await?;
            if !force {
                eprint!("Destroy VM '{vm_name}'? This cannot be undone. [y/N] ");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input).map_err(|e| {
                    tachikoma::TachikomaError::Other(format!("Failed to read input: {e}"))
                })?;
                if !input.trim().eq_ignore_ascii_case("y") {
                    print_success(mode, "Aborted", None);
                    return Ok(());
                }
            }
            tachikoma::cmd::destroy::run(&vm_name, &tart, &state_store).await?;
            print_success(mode, &format!("Destroyed {vm_name}"), None);
        }

        Some(Command::List { repo }) => {
            tachikoma::cmd::list::run(repo.as_deref(), &tart, &state_store, mode).await?;
        }

        Some(Command::Status { name }) => {
            let vm_name = resolve_vm_name(name, &git, &cwd).await?;
            tachikoma::cmd::status::run(&vm_name, &tart, &state_store, mode).await?;
        }

        Some(Command::Cd { name }) => {
            let vm_name = resolve_vm_name(name, &git, &cwd).await?;
            let state = state_store.load().await?;
            let entry = state.find_vm(&vm_name).ok_or_else(|| {
                tachikoma::TachikomaError::Vm(format!("VM '{vm_name}' not found"))
            })?;
            println!("{}", entry.worktree_path.display());
        }

        Some(Command::Prune { days, dry_run }) => {
            let prune_days = days.unwrap_or(config.prune_after_days);
            let result =
                tachikoma::cmd::prune::run(prune_days, dry_run, &tart, &state_store).await?;
            if result.pruned.is_empty() {
                print_success(mode, "No VMs to prune", None);
            } else {
                let action = if result.dry_run {
                    "Would prune"
                } else {
                    "Pruned"
                };
                for name in &result.pruned {
                    println!("{action}: {name}");
                }
            }
        }

        Some(Command::Image { action }) => match action {
            ImageAction::Pull { name } => {
                let image = name.as_deref().unwrap_or(&config.base_image);
                tachikoma::cmd::image::pull(image, &tart).await?;
                print_success(mode, &format!("Pulled image '{image}'"), None);
            }
            ImageAction::Build => {
                print_success(mode, "Image build not yet implemented", None);
            }
            ImageAction::Push { name } => {
                let image = name.as_deref().unwrap_or(&config.base_image);
                print_success(
                    mode,
                    &format!("Push not yet implemented for '{image}'"),
                    None,
                );
            }
            ImageAction::List => {
                let images = tachikoma::cmd::image::list(&tart).await?;
                for img in images {
                    println!("{img}");
                }
            }
        },

        Some(Command::Doctor) => {
            tachikoma::cmd::doctor::run(mode).await?;
        }

        Some(Command::Config { edit }) => {
            tachikoma::cmd::config::run(edit, &config_loader, repo_root, mode).await?;
        }

        Some(Command::Ssh { action }) => match action {
            SshAction::Install => {
                tachikoma::cmd::ssh_config::install(&state_store, &config.ssh_user).await?;
                print_success(mode, "SSH config installed", None);
            }
            SshAction::Uninstall => {
                tachikoma::cmd::ssh_config::uninstall().await?;
                print_success(mode, "SSH config removed", None);
            }
            SshAction::Refresh => {
                tachikoma::cmd::ssh_config::refresh(&state_store, &config.ssh_user).await?;
                print_success(mode, "SSH config refreshed", None);
            }
        },

        Some(Command::Completions { shell }) => {
            let mut cmd = Cli::command();
            clap_complete::generate(shell, &mut cmd, "tachikoma", &mut std::io::stdout());
        }

        Some(Command::Mcp) => {
            tachikoma::mcp::run_server().await?;
        }
    }

    Ok(())
}

fn make_spinner(mode: OutputMode) -> ProgressBar {
    if mode == OutputMode::Json {
        return ProgressBar::hidden();
    }
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb
}

async fn resolve_repo(
    git: &RealGitWorktree,
    cwd: &std::path::Path,
) -> tachikoma::Result<(String, std::path::PathBuf)> {
    let root = git.find_repo_root(cwd).await?;
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    Ok((name, root))
}

async fn resolve_vm_name(
    explicit: Option<String>,
    git: &RealGitWorktree,
    cwd: &std::path::Path,
) -> tachikoma::Result<String> {
    match explicit {
        Some(name) => Ok(name),
        None => {
            let branch = git.current_branch(cwd).await?;
            let (repo_name, _) = resolve_repo(git, cwd).await?;
            Ok(tachikoma::vm_name(&repo_name, &branch))
        }
    }
}
