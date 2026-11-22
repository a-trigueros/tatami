//! `tt merge [cible]` / `tt rebase [cible]` — integrate the current
//! workspace's commits into a target bookmark (jj-ized workflow, worktrunk
//! parity of intent):
//!
//! 1. resolve the target: explicit bookmark, else `main` → `master` →
//!    `trunk` (local names);
//! 2. require the **namesake bookmark** (`<workspace>`, created by
//!    `switch --create`, decision #6) — it marks the workspace's identity
//!    in `tt list` and for tooling; the merged content itself is simply
//!    "everything non-empty reachable from `<workspace>@` but not from the
//!    target" (source revset below). Bookmark position is irrelevant — no
//!    `jj bookmark set` dance before merging;
//! 3. `tt rebase` re-parents that chain onto the target (structure preserved,
//!    verified by graph on 0.47.1); `tt merge` additionally fast-forwards the
//!    target to `heads(<chain>)` — never to the empty working-copy tip.
//!    tatami only fast-forwards and verifies ancestry itself (jj's
//!    `bookmark set` does not guard).
//!
//! Merging does **not** remove the workspace (worktrunk removes it, but the
//! caller would be standing in a deleted directory — revisit with a
//! `--remove` flag once switch can move the caller). No staleness: the
//! current workspace's working copy is always updated by jj itself.

use std::io::Write;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::hook::{self, ApprovalGate, HookContext, HookError, HookPhase, Hooks};
use crate::jj::{Jj, JjError};

/// Candidate names for the default target bookmark, in order.
const DEFAULT_TARGETS: [&str; 3] = ["main", "master", "trunk"];

/// Chooses the default target: first candidate present among `available`.
pub(crate) fn default_target(available: &[String]) -> Result<String, MergeError> {
    DEFAULT_TARGETS
        .iter()
        .find(|candidate| available.iter().any(|name| name == *candidate))
        .map(|name| (*name).to_string())
        .ok_or_else(|| MergeError::NoDefaultBookmark {
            available: available.join(", "),
        })
}

/// The shared core of `merge` and `rebase`.
///
/// `move_target_bookmark` distinguishes the two: merge fast-forwards the
/// target onto the workspace's non-empty tip, rebase only re-parents the
/// chain.
pub(crate) fn integrate(
    jj: &dyn Jj,
    workspace: &str,
    target: Option<&str>,
    move_target_bookmark: bool,
) -> Result<IntegrateOutcome, MergeError> {
    let bookmarks = jj.bookmark_names()?;
    let target = match target {
        Some(explicit) => {
            if !bookmarks.iter().any(|name| name == explicit) {
                return Err(MergeError::UnknownTarget {
                    name: explicit.to_string(),
                    available: bookmarks.join(", "),
                });
            }
            explicit.to_string()
        }
        None => default_target(&bookmarks)?,
    };

    if !bookmarks.iter().any(|name| name == workspace) {
        return Err(MergeError::RequireNamesakeBookmark {
            workspace: workspace.to_string(),
            available: bookmarks.join(", "),
        });
    }

    // Everything non-empty in the workspace, minus what the target already has.
    let source = format!("::{workspace}@ ~ ::{target} & ~empty()");
    // The report and the fast-forward must agree on what "moved": heads()
    // skips the empty working-copy tip, so the moved chain ends there.
    let moved_chain = format!("heads(::{workspace}@ & ~empty()) ~ ::{target}");
    let commits = jj.revset_count(&moved_chain)?;
    if commits == 0 {
        return Err(MergeError::NoCommitsToMerge {
            workspace: workspace.to_string(),
            target: target.clone(),
        });
    }

    // Fast-forward guard: only rebase when the target is NOT already an
    // ancestor of the workspace tip.
    let target_id = jj.bookmark_commit_id(&target)?;
    let already_descendant = jj
        .revset_commit_ids(&format!("{target}::{workspace}@"))?
        .iter()
        .any(|id| id == &target_id);
    let mut rebased = false;
    if !already_descendant {
        jj.rebase(&source, &target)?;
        rebased = true;
    }

    let mut bookmark_moved = false;
    if move_target_bookmark {
        // heads() skips the empty working-copy tip; for a linear chain that
        // is exactly the last real commit.
        let tip = format!("heads(::{workspace}@ & ~empty())");
        jj.bookmark_set(&target, &tip)?;
        bookmark_moved = true;
    }

    Ok(IntegrateOutcome {
        target,
        rebased,
        bookmark_moved,
        commits,
        landing: None,
        removed_current: false,
    })
}

/// `tt merge [cible]` — rebase (if diverged) + fast-forward the target
/// bookmark, wrapped in the pre/post-merge hooks.
///
/// With `--remove`, the merged workspace is then forgotten and its directory
/// trashed, and [`IntegrateOutcome::landing`] carries where the caller
/// should end up (the target workspace's root, else the host root) — the
/// shell wrapper performs the `cd`. Guard kept even in that flow: the
/// **host** workspace is never removed (it holds the shared storage), and
/// that refusal happens before anything mutates.
#[derive(Debug, Clone)]
pub struct MergeArgs<'a> {
    pub target: Option<&'a str>,
    pub remove_current: bool,
    pub hooks: &'a Hooks<'a>,
    pub caller_root: &'a Path,
    pub logs_dir: Option<&'a Path>,
    pub state_dir: &'a Path,
    pub now_millis: u64,
}

pub fn run(
    jj: &dyn Jj,
    args: MergeArgs<'_>,
    gate: &mut dyn ApprovalGate,
    out: &mut dyn Write,
) -> Result<IntegrateOutcome, MergeError> {
    let workspaces = super::with_stale_recovery(jj, |jj| jj.workspaces())?;
    let current = workspaces
        .iter()
        .find(|w| w.is_current)
        .ok_or(MergeError::CurrentWorkspaceNotFound)?;
    let current_name = current.name.clone();
    let current_root = current.root.clone();

    // Refused before anything mutates: the host workspace holds the storage.
    if args.remove_current
        && let Some(root) = &current_root
        && crate::state::hosts_storage(root)
    {
        return Err(MergeError::RefuseHostWorkspace {
            workspace: current_name.clone(),
            storage: root.join(".jj").join("repo"),
        });
    }

    hook::run_blocking(
        &HookContext {
            phase: HookPhase::PreMerge,
            workspace: &current_name,
            workspace_path: current_root.as_deref().unwrap_or(args.caller_root),
            hooks: args.hooks,
            logs_dir: args.logs_dir,
        },
        gate,
    )
    .map_err(MergeError::Hook)?;

    let mut outcome = integrate(jj, &current_name, args.target, true)?;

    hook::run_detached(
        &HookContext {
            phase: HookPhase::PostMerge,
            workspace: &current_name,
            workspace_path: current_root.as_deref().unwrap_or(args.caller_root),
            hooks: args.hooks,
            logs_dir: args.logs_dir,
        },
        out,
        gate,
    );

    if args.remove_current {
        let landing = landing_root(jj, &outcome.target, &current_root)?;
        outcome.landing = Some(landing.clone());
        crate::cmd::remove::forget_and_trash(
            jj,
            &current_name,
            current_root.as_deref(),
            args.state_dir,
            args.now_millis,
        )?;
        outcome.removed_current = true;
    }
    Ok(outcome)
}

/// Where the caller should land after merging: the workspace named like the
/// target bookmark, else the host workspace root.
fn landing_root(
    jj: &dyn Jj,
    target: &str,
    current_root: &Option<PathBuf>,
) -> Result<PathBuf, MergeError> {
    let workspaces = jj.workspaces()?;
    let host = current_root
        .as_deref()
        .map(crate::state::host_root)
        .transpose()
        .map_err(MergeError::State)?;
    Ok(workspaces
        .iter()
        .find(|w| w.name == target)
        .and_then(|w| w.root.clone())
        .unwrap_or(host.unwrap_or_else(|| PathBuf::from("."))))
}

pub(crate) fn current_workspace_name(jj: &dyn Jj) -> Result<String, MergeError> {
    super::with_stale_recovery(jj, |jj| jj.workspaces())?
        .into_iter()
        .find(|w| w.is_current)
        .map(|w| w.name)
        .ok_or(MergeError::CurrentWorkspaceNotFound)
}

/// `tt rebase [cible]` — re-parent the workspace's commits onto the target.
pub fn rebase(jj: &dyn Jj, target: Option<&str>) -> Result<IntegrateOutcome, MergeError> {
    let workspace = current_workspace_name(jj)?;
    integrate(jj, &workspace, target, false)
}

#[derive(Debug, Error)]
pub enum MergeError {
    #[error(transparent)]
    Jj(#[from] JjError),
    #[error("cannot determine the current workspace")]
    CurrentWorkspaceNotFound,
    #[error("no default target bookmark (candidates: main, master, trunk); available: {available}")]
    NoDefaultBookmark { available: String },
    #[error("no bookmark named `{name}` (available: {available})")]
    UnknownTarget { name: String, available: String },
    #[error(
        "workspace `{workspace}` has no namesake bookmark — its commits cannot be told apart from other workspaces'. Fix: jj bookmark create {workspace} -r @{workspace}"
    )]
    RequireNamesakeBookmark {
        workspace: String,
        available: String,
    },
    #[error("nothing to merge: workspace `{workspace}` has no non-empty commit outside `{target}`")]
    NoCommitsToMerge { workspace: String, target: String },
    #[error(
        "refusing to remove `{workspace}` with --remove: it hosts the shared repo storage ({storage})"
    )]
    RefuseHostWorkspace { workspace: String, storage: PathBuf },
    #[error(transparent)]
    Hook(#[from] HookError),
    #[error(transparent)]
    Remove(#[from] crate::cmd::remove::RemoveError),
    #[error(transparent)]
    State(#[from] crate::state::StateError),
}

#[derive(Debug)]
pub struct IntegrateOutcome {
    pub target: String,
    pub rebased: bool,
    pub bookmark_moved: bool,
    pub commits: usize,
    /// Where the caller should land after `--remove`.
    pub landing: Option<PathBuf>,
    /// Set when `--remove` actually forgot the current workspace.
    pub removed_current: bool,
}

#[cfg(test)]
mod default_target_should {
    use super::*;
    use pretty_assertions::assert_eq;

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    #[test]
    fn prefers_main_then_master_then_trunk() {
        assert_eq!(
            default_target(&names(&["trunk", "master", "main"])).unwrap(),
            "main"
        );
        assert_eq!(
            default_target(&names(&["trunk", "master"])).unwrap(),
            "master"
        );
        assert_eq!(default_target(&names(&["trunk"])).unwrap(), "trunk");
    }

    #[test]
    fn fails_listing_the_available_bookmarks() {
        let error = default_target(&names(&["feature", "ws2"])).unwrap_err();
        match error {
            MergeError::NoDefaultBookmark { available } => {
                assert_eq!(available, "feature, ws2");
            }
            other => panic!("expected NoDefaultBookmark, got: {other:?}"),
        }
    }
}
