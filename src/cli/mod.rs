pub mod output;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "tachikoma",
    about = "Autonomous VM sandboxes per git worktree",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Branch to spawn (shorthand for `spawn <branch>`)
    #[arg(value_name = "BRANCH")]
    pub branch: Option<String>,

    /// Output as JSON
    #[arg(long, global = true)]
    pub json: bool,

    /// Verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Spawn a VM for a branch
    Spawn {
        /// Branch name (defaults to current branch)
        branch: Option<String>,
    },
    /// SSH into a running VM
    Enter {
        /// VM name (defaults to current branch VM)
        name: Option<String>,
    },
    /// Execute a command in the VM
    Exec {
        /// Command and arguments to execute
        #[arg(last = true)]
        cmd: Vec<String>,
    },
    /// Stop a VM
    Halt {
        /// VM name (defaults to current branch VM)
        name: Option<String>,
    },
    /// Stop a VM (alias for halt)
    Stop {
        /// VM name (defaults to current branch VM)
        name: Option<String>,
    },
    /// Suspend a VM (save state to disk)
    Suspend {
        /// VM name (defaults to current branch VM)
        name: Option<String>,
    },
    /// Destroy a VM and its state
    Destroy {
        /// VM name (defaults to current branch VM)
        name: Option<String>,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Destroy a VM and its state (alias for destroy)
    Delete {
        /// VM name (defaults to current branch VM)
        name: Option<String>,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// List all VMs
    List {
        /// Filter by repository name
        #[arg(long)]
        repo: Option<String>,
    },
    /// Show VM status
    Status {
        /// VM name (defaults to current branch VM)
        name: Option<String>,
    },
    /// Print worktree path for shell cd
    Cd {
        /// VM name (defaults to current branch VM)
        name: Option<String>,
    },
    /// Commit Claude's changes and open a GitHub PR
    Pr {
        /// VM name (defaults to current branch VM)
        #[arg(long)]
        name: Option<String>,
    },
    /// Prune old VMs
    Prune {
        /// Days since last use (default from config)
        #[arg(long)]
        days: Option<u64>,
        /// Show what would be pruned without acting
        #[arg(long)]
        dry_run: bool,
    },
    /// Manage golden images
    Image {
        #[command(subcommand)]
        action: ImageAction,
    },
    /// Run diagnostic checks
    Doctor,
    /// Show configuration
    Config {
        /// Open config in editor
        #[arg(long)]
        edit: bool,
    },
    /// Manage SSH config entries
    Ssh {
        #[command(subcommand)]
        action: SshAction,
    },
    /// Generate shell completions
    #[command(hide = true)]
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },
    /// Start MCP server (stdio JSON-RPC)
    Mcp,
    /// Manage the credential proxy
    Proxy {
        #[command(subcommand)]
        action: Option<ProxyAction>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProxyAction {
    /// Start the credential proxy (default when no action given)
    Start {
        /// Port to listen on (overrides config)
        #[arg(long)]
        port: Option<u16>,
        /// Address to bind to (overrides config)
        #[arg(long)]
        bind: Option<String>,
        /// Run as a background daemon
        #[arg(long, short)]
        daemon: bool,
    },
    /// Stop the running credential proxy
    Stop,
    /// Show credential proxy status
    Status,
}

#[derive(Subcommand, Debug)]
pub enum ImageAction {
    /// Pull base image from registry
    Pull {
        /// Image name (defaults to config base_image)
        name: Option<String>,
    },
    /// Build golden image from base
    Build {
        /// Output image name (default: tachikoma-golden)
        #[arg(long, short)]
        name: Option<String>,
        /// Overwrite existing golden image
        #[arg(long)]
        force: bool,
    },
    /// Push golden image to registry
    Push {
        /// Image name
        name: Option<String>,
    },
    /// List available images
    List,
}

#[derive(Subcommand, Debug)]
pub enum SshAction {
    /// Add VM entries to ~/.ssh/config
    Install,
    /// Remove VM entries from ~/.ssh/config
    Uninstall,
    /// Update VM entries in ~/.ssh/config
    Refresh,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_no_args() {
        let cli = Cli::try_parse_from(["tachikoma"]).unwrap();
        assert!(cli.command.is_none());
        assert!(cli.branch.is_none());
    }

    #[test]
    fn test_spawn_command() {
        let cli = Cli::try_parse_from(["tachikoma", "spawn", "main"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Spawn { branch: Some(ref b) }) if b == "main"));
    }

    #[test]
    fn test_branch_arg() {
        let cli = Cli::try_parse_from(["tachikoma", "main"]).unwrap();
        assert_eq!(cli.branch.as_deref(), Some("main"));
    }

    #[test]
    fn test_json_flag() {
        let cli = Cli::try_parse_from(["tachikoma", "--json", "list"]).unwrap();
        assert!(cli.json);
    }

    #[test]
    fn test_verbose_flag() {
        let cli = Cli::try_parse_from(["tachikoma", "-v", "doctor"]).unwrap();
        assert!(cli.verbose);
    }

    #[test]
    fn test_completions_command() {
        let cli = Cli::try_parse_from(["tachikoma", "completions", "bash"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Completions { .. })));
    }

    #[test]
    fn test_pr_command_parses() {
        let cli = Cli::try_parse_from(["tachikoma", "pr"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Pr { name: None })));
    }

    #[test]
    fn test_pr_command_with_name() {
        let cli = Cli::try_parse_from(["tachikoma", "pr", "--name", "myrepo-main"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Pr { name: Some(ref n) }) if n == "myrepo-main"
        ));
    }
}
