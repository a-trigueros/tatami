//! `tt step …` — the building blocks, runnable one at a time (worktrunk
//! parity of intent). Thin on purpose: each is one jj operation plus tatami's
//! guards (stale recovery, hooks, workspace identity).
//!
//! `copy-ignored` is intentionally absent for now: worktrunk's exact
//! semantics (source/target, file selection) still need sourcing before a
//! port — PLAN.md.

use std::io::Write;
use std::path::Path;
use std::process::Command;

use thiserror::Error;

use crate::cmd::merge::{self, MergeError};
use crate::cmd::remove::{RemoveArgs, RemoveError};
use crate::cmd::with_stale_recovery;
use crate::hook::{self, ApprovalGate, HookContext, HookError, HookPhase, Hooks};
use crate::jj::{Jj, JjError};

/// `tt step commit -m <msg>` — describe the working copy, start a new empty
/// one, wrapped in the pre/post-commit hooks.
pub fn commit(
    jj: &dyn Jj,
    message: &str,
    hooks: &Hooks<'_>,
    gate: &mut dyn ApprovalGate,
    caller_root: &Path,
    logs_dir: Option<&Path>,
    out: &mut dyn Write,
) -> Result<(), StepError> {
    let workspace = merge::current_workspace_name(jj)?;
    hook::run_blocking(
        &HookContext {
            phase: HookPhase::PreCommit,
            workspace: &workspace,
            workspace_path: caller_root,
            hooks,
            logs_dir: None,
        },
        gate,
    )
    .map_err(StepError::Hook)?;

    with_stale_recovery(jj, |jj| jj.commit(message))?;

    hook::run_detached(
        &HookContext {
            phase: HookPhase::PostCommit,
            workspace: &workspace,
            workspace_path: caller_root,
            hooks,
            logs_dir,
        },
        out,
        gate,
    );
    Ok(())
}

/// `tt step squash` — move the working-copy changes into the parent commit.
pub fn squash(jj: &dyn Jj) -> Result<(), StepError> {
    with_stale_recovery(jj, |jj| jj.squash())?;
    Ok(())
}

/// `tt step rebase [cible]` — same core as `tt rebase`.
pub fn rebase(jj: &dyn Jj, target: Option<&str>) -> Result<merge::IntegrateOutcome, MergeError> {
    merge::rebase(jj, target)
}

/// `tt step push` — push the current workspace's namesake bookmark.
pub fn push(jj: &dyn Jj, bookmark: Option<&str>) -> Result<String, StepError> {
    let name = match bookmark {
        Some(explicit) => explicit.to_string(),
        None => current_workspace(jj)?,
    };
    with_stale_recovery(jj, |jj| jj.push_bookmark(&name))?;
    Ok(name)
}

/// `tt step diff` — the working-copy diff, verbatim.
pub fn diff(jj: &dyn Jj) -> Result<String, StepError> {
    with_stale_recovery(jj, |jj| jj.diff()).map_err(StepError::Jj)
}

/// One workspace processed by [`for_each`].
#[derive(Debug)]
pub struct ForEachOutcome {
    pub workspace: String,
    pub failed: Option<String>,
}

/// `tt step for-each <cmd…>` — run `sh -c "<cmd>"` in every workspace root
/// whose directory exists (missing directories are skipped with a note).
/// Sequential on purpose: output interleaving of concurrent runs is worse
/// than the few seconds saved. shortcut: revisit if long-running commands
/// become a real use case.
pub fn for_each(
    jj: &dyn Jj,
    command: &[String],
    out: &mut dyn Write,
) -> Result<Vec<ForEachOutcome>, StepError> {
    if command.is_empty() {
        return Err(StepError::EmptyCommand);
    }
    let joined = command.join(" ");
    let workspaces = with_stale_recovery(jj, |jj| jj.workspaces())?;

    let mut outcomes = Vec::new();
    for workspace in &workspaces {
        let Some(root) = &workspace.root else {
            continue;
        };
        if !root.exists() {
            let _ = writeln!(
                out,
                "tatami: skipping `{}` (directory gone)",
                workspace.name
            );
            continue;
        }
        let status = Command::new("sh")
            .arg("-c")
            .arg(&joined)
            .current_dir(root)
            .status()
            .map_err(|source| StepError::Io { source })?;
        outcomes.push(ForEachOutcome {
            workspace: workspace.name.clone(),
            failed: if status.success() {
                None
            } else {
                Some(status.to_string())
            },
        });
    }
    Ok(outcomes)
}

/// One workspace removed by [`prune`].
#[derive(Debug)]
pub struct PrunedWorkspace {
    pub name: String,
}

/// `tt step prune` — remove every non-current workspace whose work is fully
/// integrated into the default target (nothing non-empty outside it), going
/// through the regular removal path (guards, hooks, trash, approvals).
pub fn prune(
    jj: &dyn Jj,
    hooks: &Hooks<'_>,
    gate: &mut dyn ApprovalGate,
    state_dir: &Path,
    caller_root: &Path,
    now_millis: u64,
    out: &mut dyn Write,
) -> Result<Vec<PrunedWorkspace>, StepError> {
    let workspaces = with_stale_recovery(jj, |jj| jj.workspaces())?;
    let bookmarks = jj.bookmark_names()?;
    let target = merge::default_target(&bookmarks)?;

    let mut pruned = Vec::new();
    for workspace in &workspaces {
        if workspace.is_current {
            continue;
        }
        let name = &workspace.name;
        if !bookmarks.iter().any(|bookmark| bookmark == name) {
            let _ = writeln!(
                out,
                "tatami: keeping `{}` (no namesake bookmark to judge integration)",
                name
            );
            continue;
        }
        let integrated = jj.revset_count(&format!("::{name}@ ~ ::{target} & ~empty()"))? == 0;
        if !integrated {
            continue;
        }
        match crate::cmd::remove::run(
            jj,
            RemoveArgs {
                name,
                delete_bookmark: false,
                now_millis: now_millis.wrapping_add(pruned.len() as u64),
                state_dir,
                hooks,
                caller_root,
            },
            out,
            gate,
        ) {
            Ok(_) => pruned.push(PrunedWorkspace {
                name: name.to_string(),
            }),
            Err(crate::cmd::remove::RemoveError::RefuseCurrentWorkspace { .. })
            | Err(crate::cmd::remove::RemoveError::RefuseHostWorkspace { .. }) => {
                let _ = writeln!(out, "tatami: keeping `{}` (guarded)", name);
            }
            Err(other) => return Err(other.into()),
        }
    }
    Ok(pruned)
}

fn current_workspace(jj: &dyn Jj) -> Result<String, StepError> {
    with_stale_recovery(jj, |jj| jj.workspaces())?
        .into_iter()
        .find(|w| w.is_current)
        .map(|w| w.name)
        .ok_or(StepError::CurrentWorkspaceNotFound)
}

#[derive(Debug, Error)]
pub enum StepError {
    #[error(transparent)]
    Jj(#[from] JjError),
    #[error(transparent)]
    Hook(#[from] HookError),
    #[error(transparent)]
    Merge(#[from] MergeError),
    #[error(transparent)]
    Remove(#[from] RemoveError),
    #[error("filesystem error: {source}")]
    Io {
        #[source]
        source: std::io::Error,
    },
    #[error("`for-each` needs a command to run")]
    EmptyCommand,
    #[error("cannot determine the current workspace")]
    CurrentWorkspaceNotFound,
}
