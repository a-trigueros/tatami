//! CLI definition (clap). Lives in the library so the shell-completion
//! generator and the binary share one source of truth.

use clap::{Parser, Subcommand, ValueEnum};
use std::ffi::OsString;

/// Output formats for commands that support `--format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ListFormat {
    Text,
    Json,
}

impl From<ListFormat> for crate::cmd::list::Format {
    fn from(value: ListFormat) -> Self {
        match value {
            ListFormat::Text => Self::Text,
            ListFormat::Json => Self::Json,
        }
    }
}

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
    /// List all workspaces with their working-copy status
    List {
        /// Output format
        #[arg(long, value_enum, default_value = "text")]
        format: ListFormat,
    },
    /// Resolve the workspace to switch to (the shell wrapper performs the cd)
    Switch {
        /// Workspace name, or `-` for the previous one; omitted for the picker
        name: Option<String>,
        /// Create the workspace (and its namesake bookmark) before switching
        #[arg(short = 'c', long)]
        create: bool,
    },
    /// Forget a workspace and move its directory to the tatami trash
    Remove {
        /// Workspace to remove (cannot be the current one or the host of the
        /// shared repo storage)
        name: String,
        /// Keep the namesake bookmark (deleted by default)
        #[arg(long = "keep-bookmark")]
        keep_bookmark: bool,
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
        /// Also forget the merged workspace (the wrapper cds to the printed
        /// destination; without the wrapper, run from another workspace)
        #[arg(long)]
        remove: bool,
    },
    /// Building blocks, runnable one at a time
    Step {
        #[command(subcommand)]
        command: StepCommand,
    },
    /// Run a configured alias (`[aliases]` in the config) — everything else
    /// lands here as an external subcommand
    #[command(external_subcommand)]
    Unknown(Vec<OsString>),
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
    /// Inspect or purge tatami's internal state (trash, logs)
    State {
        #[command(subcommand)]
        command: StateCommand,
    },
    /// Record a project-hook approval ahead of time (CI, unattended runs)
    Approvals {
        #[command(subcommand)]
        command: ApprovalsCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum StateCommand {
    /// Print the state directory and its trash contents
    Show,
    /// Empty the trash — destructive: trashed directories may hold ignored
    /// files that exist nowhere else
    Clear {
        /// Actually delete (without it, only lists what would go)
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ApprovalsCommand {
    /// Approve a project hook command for the current project
    Add {
        /// Lifecycle phase (pre-switch, post-switch, pre-start, post-start,
        /// pre-remove, post-remove, pre-merge, post-merge, pre-commit,
        /// post-commit)
        phase: String,
        /// The hook command, exactly as written in the project config
        command: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum StepCommand {
    /// Describe the working copy and start a new empty one
    Commit {
        /// Commit description
        #[arg(short, long)]
        message: String,
    },
    /// Move the working-copy changes into the parent commit
    Squash,
    /// Re-parent the workspace commits onto a target (same as `tt rebase`)
    Rebase {
        /// Target bookmark; defaults to main, then master, then trunk
        target: Option<String>,
    },
    /// Push the workspace's namesake bookmark
    Push,
    /// Show the working-copy diff
    Diff,
    /// Run a command in every workspace
    ForEach {
        /// Command to run (executed via sh, cwd = each workspace)
        command: Vec<String>,
    },
    /// Remove workspaces whose commits are fully integrated
    Prune,
    /// Copy git-ignored files from one workspace to another
    CopyIgnored {
        /// Source workspace; defaults to the primary (host) workspace
        #[arg(long)]
        from: Option<String>,
        /// Destination workspace; defaults to the current one
        #[arg(long)]
        to: Option<String>,
        /// Overwrite destination files that already exist
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ShellCommand {
    /// Print the shell integration block (wrapper `tt` + completions) to add
    /// to your rc file — tatami never writes rc files itself
    Init {
        /// Target shell (zsh, bash); defaults to $SHELL
        shell: Option<String>,
    },
    /// Print the completion script for the given shell
    Completion {
        /// Target shell (zsh, bash)
        shell: String,
    },
}

#[cfg(test)]
mod cli_should {
    use super::*;

    #[test]
    fn switch_accepts_c_as_the_short_form_of_create() {
        let cli = Cli::try_parse_from(["tatami", "switch", "-c", "toto"]).expect("parse -c");
        match cli.command {
            Command::Switch { name, create } => {
                assert!(create, "-c must set create");
                assert_eq!(name.as_deref(), Some("toto"));
            }
            other => panic!("expected Switch, got: {other:?}"),
        }
    }

    #[test]
    fn remove_deletes_the_bookmark_by_default_and_keep_bookmark_opts_out() {
        let cli = Cli::try_parse_from(["tatami", "remove", "ws2"]).expect("parse remove");
        match cli.command {
            Command::Remove { keep_bookmark, .. } => {
                assert!(!keep_bookmark, "the bookmark is deleted by default");
            }
            other => panic!("expected Remove, got: {other:?}"),
        }

        let cli = Cli::try_parse_from(["tatami", "remove", "ws2", "--keep-bookmark"])
            .expect("parse --keep-bookmark");
        match cli.command {
            Command::Remove { keep_bookmark, .. } => {
                assert!(keep_bookmark, "--keep-bookmark must keep it");
            }
            other => panic!("expected Remove, got: {other:?}"),
        }
    }
}
