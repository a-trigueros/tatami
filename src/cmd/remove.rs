//! `tt remove <nom>` — forget a workspace and dispose of its directory.
//!
//! Guards, in order:
//! 1. the workspace must exist (error lists the available ones);
//! 2. the **current** workspace cannot be removed — tatami cannot move the
//!    caller out of the directory it is about to delete;
//! 3. the workspace **hosting** the shared repo storage cannot be removed —
//!    deleting that directory destroys the storage every other workspace
//!    points at (see PLAN.md §5). No flag overrides these two.
//!
//! The directory is not `rm -rf`-ed: it is moved into the tatami state
//! directory's `trash/` (deferred deletion, see PLAN.md), because ignored
//! files (`.env`…) live only there. When the move would cross a filesystem
//! boundary (EXDEV), the directory is deleted in place instead.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::hook::{self, ApprovalGate, HookContext, HookError, HookPhase, Hooks};
use crate::jj::{Jj, JjError, WorkspaceInfo};
use crate::state;

pub struct RemoveArgs<'a> {
    pub name: &'a str,
    pub delete_bookmark: bool,
    /// Injected clock (unix millis) for unique trash entries.
    pub now_millis: u64,
    /// The repo-global tatami state dir, hosting `trash/`.
    pub state_dir: &'a Path,
    /// Hook sources (user first, project second, approval-gated).
    pub hooks: &'a Hooks<'a>,
    /// The workspace tatami runs in — post-remove's cwd, since the removed
    /// directory is gone by then.
    pub caller_root: &'a Path,
}

pub fn run(
    jj: &dyn Jj,
    args: RemoveArgs<'_>,
    out: &mut dyn Write,
    gate: &mut dyn ApprovalGate,
) -> Result<RemoveReport, RemoveError> {
    let workspaces = crate::cmd::with_stale_recovery(jj, |jj| jj.workspaces())?;
    let target = resolve(&workspaces, args.name)?;
    if target.is_current {
        return Err(RemoveError::RefuseCurrentWorkspace {
            name: args.name.to_string(),
        });
    }
    if let Some(root) = &target.root
        && state::hosts_storage(root)
    {
        return Err(RemoveError::RefuseHostWorkspace {
            name: args.name.to_string(),
            storage: root.join(".jj").join("repo"),
        });
    }

    // pre-remove runs in the doomed directory while it still exists.
    if let Some(root) = &target.root {
        hook::run_blocking(
            &HookContext {
                phase: HookPhase::PreRemove,
                workspace: args.name,
                workspace_path: root,
                hooks: args.hooks,
            },
            gate,
        )
        .map_err(RemoveError::Hook)?;
    }

    jj.workspace_forget(args.name)?;
    let bookmark_deleted = if args.delete_bookmark {
        jj.bookmark_delete(args.name)?
    } else {
        false
    };

    let directory = match &target.root {
        Some(root) if root.exists() => {
            DirectoryOutcome::Trashed(trash(root, args.state_dir, args.name, args.now_millis)?)
        }
        _ => DirectoryOutcome::AlreadyGone,
    };

    hook::run_detached(
        &HookContext {
            phase: HookPhase::PostRemove,
            workspace: args.name,
            workspace_path: args.caller_root,
            hooks: args.hooks,
        },
        out,
        gate,
    );

    Ok(RemoveReport {
        name: args.name.to_string(),
        bookmark_deleted,
        directory,
    })
}

fn resolve<'w>(
    workspaces: &'w [WorkspaceInfo],
    name: &str,
) -> Result<&'w WorkspaceInfo, RemoveError> {
    workspaces.iter().find(|w| w.name == name).ok_or_else(|| {
        let mut available = workspaces
            .iter()
            .map(|w| w.name.as_str())
            .collect::<Vec<_>>();
        available.sort();
        RemoveError::UnknownWorkspace {
            name: name.to_string(),
            available: available.join(", "),
        }
    })
}

fn trash(
    root: &Path,
    state_dir: &Path,
    name: &str,
    now_millis: u64,
) -> Result<PathBuf, RemoveError> {
    // Unix errno EXDEV (18): rename(2) across filesystems.
    const EXDEV: i32 = 18;
    let trash_dir = state_dir.join("trash");
    fs::create_dir_all(&trash_dir).map_err(|source| RemoveError::Io { source })?;
    let destination = trash_dir.join(format!("{name}-{now_millis}"));
    match fs::rename(root, &destination) {
        Ok(()) => Ok(destination),
        // Cross-device move: no trash, delete in place (data-loss risk for
        // ignored files only — tracked history lives in the repo).
        Err(source) if source.raw_os_error() == Some(EXDEV) => {
            fs::remove_dir_all(root).map_err(|source| RemoveError::Io { source })?;
            Ok(destination)
        }
        Err(source) => Err(RemoveError::Io { source }),
    }
}

#[derive(Debug)]
pub struct RemoveReport {
    pub name: String,
    pub bookmark_deleted: bool,
    pub directory: DirectoryOutcome,
}

#[derive(Debug)]
pub enum DirectoryOutcome {
    /// Moved into the tatami trash, still recoverable until cleaned.
    Trashed(PathBuf),
    /// Nothing on disk to dispose of (already gone, or root not recorded by
    /// a pre-0.38 workspace).
    AlreadyGone,
}

#[derive(Debug, Error)]
pub enum RemoveError {
    #[error(transparent)]
    Jj(#[from] JjError),
    #[error("no workspace named `{name}` (available: {available})")]
    UnknownWorkspace { name: String, available: String },
    #[error(
        "refusing to remove `{name}`: it is the workspace you are in — switch to another one first"
    )]
    RefuseCurrentWorkspace { name: String },
    #[error(
        "refusing to remove `{name}`: it hosts the shared repo storage ({storage}) — deleting it would destroy every other workspace"
    )]
    RefuseHostWorkspace { name: String, storage: PathBuf },
    #[error("filesystem error: {source}")]
    Io {
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Hook(#[from] HookError),
}

#[cfg(test)]
mod remove_should {
    use super::*;
    use crate::hook::NoGate;
    use crate::jj::WcStatus;
    use std::cell::RefCell;

    struct FakeJj {
        workspaces: Vec<WorkspaceInfo>,
        forget_calls: RefCell<Vec<String>>,
        bookmark_delete_calls: RefCell<Vec<String>>,
        bookmark_delete_result: bool,
    }
    impl FakeJj {
        fn with_workspaces(workspaces: Vec<WorkspaceInfo>) -> Self {
            Self {
                workspaces,
                forget_calls: RefCell::new(Vec::new()),
                bookmark_delete_calls: RefCell::new(Vec::new()),
                bookmark_delete_result: true,
            }
        }
    }

    impl Jj for FakeJj {
        fn workspace_root(&self) -> Result<PathBuf, JjError> {
            panic!("not exercised by remove tests")
        }

        fn workspaces(&self) -> Result<Vec<WorkspaceInfo>, JjError> {
            Ok(self.workspaces.clone())
        }

        fn workspace_add(&self, _name: &str, _destination: &Path) -> Result<(), JjError> {
            panic!("not exercised by remove tests")
        }

        fn bookmark_create(&self, _name: &str, _workspace: &str) -> Result<(), JjError> {
            panic!("not exercised by remove tests")
        }

        fn workspace_forget(&self, name: &str) -> Result<(), JjError> {
            self.forget_calls.borrow_mut().push(name.to_string());
            Ok(())
        }

        fn bookmark_delete(&self, name: &str) -> Result<bool, JjError> {
            self.bookmark_delete_calls
                .borrow_mut()
                .push(name.to_string());
            Ok(self.bookmark_delete_result)
        }

        fn bookmark_names(&self) -> Result<Vec<String>, JjError> {
            panic!("FakeJj::bookmark_names is not exercised by remove tests")
        }

        fn bookmark_commit_id(&self, _name: &str) -> Result<String, JjError> {
            panic!("FakeJj::bookmark_commit_id is not exercised by remove tests")
        }

        fn revset_count(&self, _revset: &str) -> Result<usize, JjError> {
            panic!("FakeJj::revset_count is not exercised by remove tests")
        }

        fn revset_commit_ids(&self, _revset: &str) -> Result<Vec<String>, JjError> {
            panic!("FakeJj::revset_commit_ids is not exercised by remove tests")
        }

        fn rebase(&self, _source: &str, _destination: &str) -> Result<(), JjError> {
            panic!("FakeJj::rebase is not exercised by remove tests")
        }

        fn bookmark_set(&self, _name: &str, _revision: &str) -> Result<(), JjError> {
            panic!("FakeJj::bookmark_set is not exercised by remove tests")
        }

        fn workspace_update_stale(&self) -> Result<(), JjError> {
            panic!("FakeJj::workspace_update_stale is not exercised by remove tests")
        }
    }

    fn ws(name: &str, current: bool, root: Option<PathBuf>) -> WorkspaceInfo {
        WorkspaceInfo {
            name: name.to_string(),
            root,
            is_current: current,
            wc: WcStatus {
                change: "szyooy".to_string(),
                description: String::new(),
                empty: true,
                conflict: false,
                divergent: false,
            },
        }
    }

    fn remove_from(
        jj: &dyn Jj,
        name: &str,
        delete_bookmark: bool,
        state_dir: &Path,
    ) -> Result<RemoveReport, RemoveError> {
        let mut sink = Vec::new();
        let hooks = Hooks::none();
        let mut gate = NoGate;
        run(
            jj,
            RemoveArgs {
                name,
                delete_bookmark,
                now_millis: 1_000,
                state_dir,
                hooks: &hooks,
                caller_root: Path::new("/tmp/caller"),
            },
            &mut sink,
            &mut gate,
        )
    }

    #[test]
    fn forgets_a_workspace_and_trashes_its_directory() {
        let parent = tempfile::tempdir().unwrap();
        let ws2_root = parent.path().join("ws2");
        fs::create_dir_all(ws2_root.join("src")).unwrap();
        fs::write(ws2_root.join("src/main.rs"), "fn main() {}").unwrap();
        let state_dir = tempfile::tempdir().unwrap();
        let jj = FakeJj::with_workspaces(vec![
            ws("default", true, Some(parent.path().join("main"))),
            ws("ws2", false, Some(ws2_root.clone())),
        ]);

        let report = remove_from(&jj, "ws2", false, state_dir.path()).expect("remove ws2");

        assert_eq!(*jj.forget_calls.borrow(), vec!["ws2".to_string()]);
        let trashed = match report.directory {
            DirectoryOutcome::Trashed(path) => path,
            other => panic!("expected Trashed, got: {other:?}"),
        };
        assert!(
            trashed.starts_with(state_dir.path().join("trash")),
            "trashed path was: {trashed:?}"
        );
        assert!(
            trashed.join("src/main.rs").exists(),
            "contents preserved in trash"
        );
        assert!(!ws2_root.exists(), "origin directory must be gone");
    }

    #[test]
    fn keeps_the_bookmark_by_default() {
        let parent = tempfile::tempdir().unwrap();
        let state_dir = tempfile::tempdir().unwrap();
        let jj = FakeJj::with_workspaces(vec![
            ws("default", true, Some(parent.path().join("main"))),
            ws("ws2", false, Some(parent.path().join("ws2"))),
        ]);

        remove_from(&jj, "ws2", false, state_dir.path()).unwrap();

        assert!(jj.bookmark_delete_calls.borrow().is_empty());
    }

    #[test]
    fn deletes_the_bookmark_when_asked() {
        let parent = tempfile::tempdir().unwrap();
        let state_dir = tempfile::tempdir().unwrap();
        let jj = FakeJj::with_workspaces(vec![
            ws("default", true, Some(parent.path().join("main"))),
            ws("ws2", false, Some(parent.path().join("ws2"))),
        ]);

        let report = remove_from(&jj, "ws2", true, state_dir.path()).unwrap();

        assert_eq!(*jj.bookmark_delete_calls.borrow(), vec!["ws2".to_string()]);
        assert!(report.bookmark_deleted);
    }

    #[test]
    fn refuses_to_remove_the_workspace_we_are_in() {
        let parent = tempfile::tempdir().unwrap();
        let state_dir = tempfile::tempdir().unwrap();
        let jj = FakeJj::with_workspaces(vec![
            ws("default", true, Some(parent.path().join("main"))),
            ws("ws2", false, Some(parent.path().join("ws2"))),
        ]);

        let error = remove_from(&jj, "default", false, state_dir.path()).unwrap_err();

        assert!(
            matches!(error, RemoveError::RefuseCurrentWorkspace { .. }),
            "got: {error:?}"
        );
        assert!(
            jj.forget_calls.borrow().is_empty(),
            "nothing must be forgotten"
        );
    }

    #[test]
    fn refuses_to_remove_the_host_of_the_shared_storage() {
        let parent = tempfile::tempdir().unwrap();
        let main = parent.path().join("main");
        let ws2 = parent.path().join("ws2");
        fs::create_dir_all(main.join(".jj/repo")).unwrap();
        fs::create_dir_all(&ws2).unwrap();
        let state_dir = tempfile::tempdir().unwrap();
        let jj = FakeJj::with_workspaces(vec![
            ws("default", false, Some(main.clone())),
            ws("ws2", true, Some(ws2)),
        ]);

        let error = remove_from(&jj, "default", false, state_dir.path()).unwrap_err();

        match error {
            RemoveError::RefuseHostWorkspace { name, storage } => {
                assert_eq!(name, "default");
                assert_eq!(storage, main.join(".jj/repo"));
            }
            other => panic!("expected RefuseHostWorkspace, got: {other:?}"),
        }
    }

    #[test]
    fn forgets_even_when_the_directory_is_already_gone() {
        let parent = tempfile::tempdir().unwrap();
        let state_dir = tempfile::tempdir().unwrap();
        let jj = FakeJj::with_workspaces(vec![
            ws("default", true, Some(parent.path().join("main"))),
            ws("ws2", false, Some(parent.path().join("removed"))),
        ]);

        let report = remove_from(&jj, "ws2", false, state_dir.path()).expect("remove ws2");

        assert_eq!(*jj.forget_calls.borrow(), vec!["ws2".to_string()]);
        assert!(matches!(report.directory, DirectoryOutcome::AlreadyGone));
    }

    #[test]
    fn fails_for_an_unknown_workspace() {
        let state_dir = tempfile::tempdir().unwrap();
        let jj = FakeJj::with_workspaces(vec![ws("default", true, None)]);

        let error = remove_from(&jj, "nope", false, state_dir.path()).unwrap_err();

        match error {
            RemoveError::UnknownWorkspace { name, available } => {
                assert_eq!(name, "nope");
                assert_eq!(available, "default");
            }
            other => panic!("expected UnknownWorkspace, got: {other:?}"),
        }
    }
}
