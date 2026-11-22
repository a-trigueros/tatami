//! CLI definition (clap). Lives in the library so the shell-completion
//! generator and the binary share one source of truth.

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "tatami", version, about = "Tatami CLI")]
pub struct Cli {
    /// Skip approval prompts for project hooks (consent for this invocation)
    #[arg(long, short = 'y', global = true)]
    pub yes: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Print a greeting
    Hello {
        /// Name to greet
        #[arg(default_value = "world")]
        name: String,
    },
    /// List all workspaces with their working-copy status
    List,
    /// Resolve the workspace to switch to (the shell wrapper performs the cd)
    Switch {
        /// Workspace name, or `-` for the previous one; omitted for the picker
        name: Option<String>,
        /// Create the workspace (and its namesake bookmark) before switching
        #[arg(long)]
        create: bool,
    },
    /// Forget a workspace and move its directory to the tatami trash
    Remove {
        /// Workspace to remove (cannot be the current one or the host of the
        /// shared repo storage)
        name: String,
        /// Also delete the namesake bookmark
        #[arg(long = "delete-bookmark")]
        delete_bookmark: bool,
    },
    /// Re-parent the current workspace's commits onto the target bookmark
    Rebase {
        /// Target bookmark; defaults to main, then master, then trunk
        target: Option<String>,
    },
    /// Integrate the current workspace into the target bookmark (rebase if
    /// diverged, then fast-forward)
    Merge {
        /// Target bookmark; defaults to main, then master, then trunk
        target: Option<String>,
    },
    /// Inspect tatami configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Print the resolved user config location, its source, and the effective
    /// workspace-path template
    Show,
    /// Manage the shell integration (wrapper `tt` + completions)
    Shell {
        #[command(subcommand)]
        command: ShellCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum ShellCommand {
    /// Add the `tt` wrapper (cd on switch) and completions to your rc file
    Install {
        /// Target shell (zsh, bash); defaults to $SHELL
        #[arg(long)]
        shell: Option<String>,
    },
    /// Remove the integration block from your rc file
    Uninstall {
        /// Target shell (zsh, bash); defaults to $SHELL
        #[arg(long)]
        shell: Option<String>,
    },
    /// Print the completion script for the given shell
    Completion {
        /// Target shell (zsh, bash)
        shell: String,
    },
}
