//! Adapter to the `jj` CLI — the only place tatami invokes jujutsu.
//!
//! jj has no stable Rust library: the subprocess interface with template
//! output (`-T`) is the only durable contract. Everything else in tatami
//! depends on the [`Jj`] trait, never on jj internals.

use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

/// Operations tatami needs from jujutsu. Faked in fast tests, proven against
/// the real binary in integration tests.
pub trait Jj {
    /// Absolute root directory of the current workspace.
    fn workspace_root(&self) -> Result<PathBuf, JjError>;

    /// All workspaces tracked by the repository, with their working-copy
    /// status.
    fn workspaces(&self) -> Result<Vec<WorkspaceInfo>, JjError>;

    /// Creates a workspace named `name` at `destination` (which must not
    /// exist; its parent must). The new working-copy commit is parented on
    /// the current one.
    fn workspace_add(&self, name: &str, destination: &Path) -> Result<(), JjError>;

    /// Creates a bookmark `name` pointing at `<workspace>@` — the new
    /// workspace's working-copy commit. A fresh `@` is empty; jj warns about
    /// it and succeeds.
    fn bookmark_create(&self, name: &str, workspace: &str) -> Result<(), JjError>;

    /// Stops tracking a workspace. jj leaves its directory untouched —
    /// removal of the directory is the caller's decision.
    fn workspace_forget(&self, name: &str) -> Result<(), JjError>;

    /// Deletes a local bookmark. Deleting an absent bookmark succeeds in jj
    /// (warning only); the returned bool says whether one was actually
    /// deleted. shortcut: detected via jj's stderr string
    /// "No matching bookmarks" — revalidate on jj upgrades.
    fn bookmark_delete(&self, name: &str) -> Result<bool, JjError>;
}

/// Working-copy commit status of a workspace, as jj sees it.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct WcStatus {
    /// Short change id, for display.
    pub change: String,
    /// Commit description, trimmed; empty when undescribed.
    pub description: String,
    /// The commit modifies no files.
    pub empty: bool,
    /// The commit contains merge conflicts.
    pub conflict: bool,
    /// The change id corresponds to multiple visible commits.
    pub divergent: bool,
}

/// A workspace tracked by the repository.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct WorkspaceInfo {
    pub name: String,
    /// Root directory as recorded by jj — canonical, symlinks resolved.
    /// `None` for workspaces created before jj 0.38.0, which did not record it.
    pub root: Option<PathBuf>,
    /// True when this is the workspace the adapter runs in.
    #[serde(rename = "current")]
    pub is_current: bool,
    /// jj's JSONL output is flat; the workspace-level vs working-copy-level
    /// split is tatami's domain structure.
    #[serde(flatten)]
    pub wc: WcStatus,
}

/// Invokes the real `jj` binary.
#[derive(Debug, Clone)]
pub struct CliJj {
    working_dir: PathBuf,
}

impl CliJj {
    /// Adapter rooted at `working_dir`: every jj invocation runs from there.
    pub fn at(working_dir: PathBuf) -> Self {
        Self { working_dir }
    }

    /// Runs `jj <args>` from the adapter's working directory and returns the
    /// trimmed stdout and stderr on success.
    fn execute(&self, args: &[&str]) -> Result<(String, String), JjError> {
        let output = Command::new("jj")
            .args(args)
            .current_dir(&self.working_dir)
            .output()
            .map_err(|source| JjError::Spawn { source })?;
        if !output.status.success() {
            return Err(JjError::CommandFailed {
                command: command_label(args),
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        Ok((
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }

    fn run(&self, args: &[&str]) -> Result<String, JjError> {
        self.execute(args).map(|(stdout, _)| stdout)
    }

    fn run_with_stderr(&self, args: &[&str]) -> Result<(String, String), JjError> {
        self.execute(args)
    }
}

/// `jj <args>` with long arguments (templates) elided, so error messages stay
/// readable while keeping the subcommand identifiable.
fn command_label(args: &[&str]) -> String {
    const MAX_ARG_CHARS: usize = 60;
    let elided: Vec<String> = args
        .iter()
        .map(|arg| {
            if arg.chars().count() > MAX_ARG_CHARS {
                let prefix: String = arg.chars().take(MAX_ARG_CHARS).collect();
                format!("{prefix}…")
            } else {
                (*arg).to_string()
            }
        })
        .collect();
    format!("jj {}", elided.join(" "))
}

/// Template emitting one JSON object per workspace. Field names must match
/// [`WorkspaceInfo`]/[`WcStatus`] serde names. Contract pinned against
/// jj 0.47.1: `WorkspaceRef` keywords `name`, `current_workspace`, `root`,
/// `target` (see docs.jj-vcs.dev templates, WorkspaceRef type).
const WORKSPACE_LIST_TEMPLATE: &str = concat!(
    r#"json({"name" => name, "current" => current_workspace, "root" => root, "#,
    r#""empty" => target.empty(), "conflict" => target.conflict(), "#,
    r#""divergent" => target.divergent(), "description" => target.description().trim_end(), "#,
    r#""change" => target.change_id().short()}) ++ "\n""#,
);

impl Jj for CliJj {
    fn workspace_root(&self) -> Result<PathBuf, JjError> {
        let root = self.run(&["workspace", "root"])?;
        Ok(PathBuf::from(root))
    }

    fn workspaces(&self) -> Result<Vec<WorkspaceInfo>, JjError> {
        let output = self.run(&["workspace", "list", "-T", WORKSPACE_LIST_TEMPLATE])?;
        let parsed: Result<Vec<WorkspaceInfo>, _> =
            output.lines().map(serde_json::from_str).collect();
        parsed.map_err(|source| JjError::ParseOutput {
            command: "jj workspace list".to_string(),
            source,
        })
    }

    fn workspace_add(&self, name: &str, destination: &Path) -> Result<(), JjError> {
        self.run(&[
            "workspace",
            "add",
            "--name",
            name,
            &destination.display().to_string(),
        ])?;
        Ok(())
    }

    fn bookmark_create(&self, name: &str, workspace: &str) -> Result<(), JjError> {
        self.run(&["bookmark", "create", name, "-r", &format!("{workspace}@")])?;
        Ok(())
    }

    fn workspace_forget(&self, name: &str) -> Result<(), JjError> {
        self.run(&["workspace", "forget", name])?;
        Ok(())
    }

    fn bookmark_delete(&self, name: &str) -> Result<bool, JjError> {
        let (_stdout, stderr) = self.run_with_stderr(&["bookmark", "delete", name])?;
        Ok(!stderr.contains("No matching bookmarks"))
    }
}

#[derive(Debug, Error)]
pub enum JjError {
    /// `jj` could not be launched at all (missing binary, permissions).
    #[error("cannot run jj (is it installed and on PATH?): {source}")]
    Spawn {
        #[source]
        source: std::io::Error,
    },
    /// jj ran and rejected the operation; stderr is carried so callers can
    /// surface jj's own diagnosis.
    #[error("`{command}` failed with {status}\n{stderr}")]
    CommandFailed {
        command: String,
        status: std::process::ExitStatus,
        stderr: String,
    },
    /// jj ran successfully but its output is not the JSONL contract tatami
    /// pinned (jj version drift).
    #[error("jj output for `{command}` does not match the expected JSONL contract: {source}")]
    ParseOutput {
        command: String,
        #[source]
        source: serde_json::Error,
    },
}
