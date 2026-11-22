//! Tatami's internal state location (see PLAN.md §5).
//!
//! State is repo-global, so it lives in the `.jj/tatami/` directory of the
//! workspace **hosting** the shared jj repo storage:
//! - in the host workspace itself, `.jj/repo` is a directory;
//! - in secondary workspaces, `.jj/repo` is a pointer *file* whose content is
//!   a path relative to that workspace's `.jj` directory (jj-private format,
//!   verified empirically on jj 0.47.1).

use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Tatami's repo-global state, persisted under the host workspace's
/// `.jj/tatami/` directory. Faked in fast tests, proven against real repos in
/// integration tests.
pub trait WorkspaceState {
    /// The workspace a successful `tt switch -` goes back to.
    fn previous_workspace(&self) -> Result<Option<String>, StateError>;

    /// Records (or clears, with `None`) the previous workspace.
    fn set_previous_workspace(&self, name: Option<&str>) -> Result<(), StateError>;
}

/// File-backed [`WorkspaceState`] for the repo reachable from a workspace.
pub struct FileState {
    dir: PathBuf,
}

/// Name of the previous-workspace file inside the state directory.
const PREVIOUS_FILE: &str = "previous_workspace";

impl FileState {
    /// Resolves the state directory for `workspace_root` (see [`state_dir`]).
    /// The directory itself is created lazily, on first write.
    pub fn for_workspace(workspace_root: &Path) -> Result<Self, StateError> {
        Ok(Self {
            dir: state_dir(workspace_root)?,
        })
    }

    fn previous_path(&self) -> PathBuf {
        self.dir.join(PREVIOUS_FILE)
    }
}

impl WorkspaceState for FileState {
    fn previous_workspace(&self) -> Result<Option<String>, StateError> {
        let path = self.previous_path();
        match fs::read_to_string(&path) {
            Ok(name) => {
                let name = name.trim().to_string();
                if name.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(name))
                }
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(StateError::ReadFile { path, source }),
        }
    }

    fn set_previous_workspace(&self, name: Option<&str>) -> Result<(), StateError> {
        let path = self.previous_path();
        fs::create_dir_all(&self.dir).map_err(|source| StateError::WriteDir {
            path: self.dir.clone(),
            source,
        })?;
        match name {
            Some(name) => fs::write(&path, format!("{name}\n"))
                .map_err(|source| StateError::WriteFile { path, source }),
            None => match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(source) => Err(StateError::WriteFile { path, source }),
            },
        }
    }
}

/// The workspace hosting the shared repo storage, reachable from
/// `workspace_root`. Stable identity for the whole project across workspaces
/// (state, approvals).
pub fn host_root(workspace_root: &Path) -> Result<PathBuf, StateError> {
    let repo_probe = workspace_root.join(".jj").join("repo");
    if repo_probe.is_dir() {
        return Ok(dunce::simplified(workspace_root).to_path_buf());
    }
    if !repo_probe.is_file() {
        return Err(StateError::NoRepoStorage {
            root: workspace_root.to_path_buf(),
        });
    }

    let pointer = fs::read_to_string(&repo_probe).map_err(|source| StateError::ReadPointer {
        path: repo_probe.clone(),
        source,
    })?;
    let storage = workspace_root
        .join(".jj")
        .join(pointer.trim())
        .canonicalize()
        .map_err(|source| StateError::DanglingPointer {
            path: repo_probe.clone(),
            source,
        })?;
    let root = storage
        .parent()
        .and_then(|jj_dir| jj_dir.parent())
        .map(Path::to_path_buf)
        .map(|root| dunce::simplified(&root).to_path_buf())
        .ok_or_else(|| StateError::MalformedPointer {
            path: repo_probe.clone(),
        })?;
    Ok(root)
}

/// Absolute directory where tatami's repo-global state lives for the repo
/// reachable from `workspace_root`.
///
/// `workspace_root` must be an existing, canonical workspace root — the shape
/// jj itself reports (`jj workspace root` resolves symlinks). Non-canonical
/// inputs would leak symlink differences into the output.
pub fn state_dir(workspace_root: &Path) -> Result<PathBuf, StateError> {
    Ok(host_root(workspace_root)?.join(".jj").join("tatami"))
}

/// True when `workspace_root` hosts the shared repo storage — the workspace
/// whose `.jj/repo` is the real directory, not a pointer file. Removing such
/// a workspace's directory would destroy the storage every other workspace
/// points to.
pub fn hosts_storage(workspace_root: &Path) -> bool {
    workspace_root.join(".jj").join("repo").is_dir()
}

/// Where removed workspace directories wait for disposal.
pub fn trash_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("trash")
}

/// Contents of the trash, oldest first. An absent trash directory is normal.
pub fn trash_entries(state_dir: &Path) -> Result<Vec<PathBuf>, StateError> {
    let dir = trash_dir(state_dir);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|source| StateError::ReadFile {
            path: dir.clone(),
            source,
        })?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    Ok(entries)
}

/// Empties the trash — destructive: trashed directories hold files that may
/// exist nowhere else (ignored files like `.env`). Callers must guard.
pub fn clear_trash(state_dir: &Path) -> Result<(), StateError> {
    let dir = trash_dir(state_dir);
    if !dir.is_dir() {
        return Ok(());
    }
    fs::remove_dir_all(&dir).map_err(|source| StateError::WriteDir { path: dir, source })
}

#[derive(Debug, Error)]
pub enum StateError {
    /// Not a workspace at all (no `.jj/repo`, neither directory nor pointer).
    #[error("no jj repo storage under {root}")]
    NoRepoStorage { root: PathBuf },
    #[error("cannot read jj repo pointer {path}: {source}")]
    ReadPointer {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The pointer file exists but its target storage is gone — typically the
    /// host workspace directory was removed.
    #[error("jj repo pointer {path} leads to missing storage: {source}")]
    DanglingPointer {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("jj repo pointer {path} does not lead to a workspace root")]
    MalformedPointer { path: PathBuf },
    #[error("cannot create tatami state directory {path}: {source}")]
    WriteDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot write tatami state file {path}: {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot read tatami state file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod state_dir_should {
    use super::*;
    use pretty_assertions::assert_eq;

    fn mkdir_all(path: &Path) {
        fs::create_dir_all(path).unwrap();
    }

    #[test]
    fn returns_the_local_tatami_dir_when_this_workspace_hosts_the_storage() {
        let root = tempfile::tempdir().unwrap();
        mkdir_all(&root.path().join(".jj/repo"));

        let state = state_dir(root.path()).unwrap();

        assert_eq!(state, root.path().join(".jj/tatami"));
    }

    #[test]
    fn follows_the_pointer_file_to_the_host_workspace() {
        let parent = tempfile::tempdir().unwrap();
        mkdir_all(&parent.path().join("main/.jj/repo"));
        mkdir_all(&parent.path().join("ws2/.jj"));
        fs::write(parent.path().join("ws2/.jj/repo"), "../../main/.jj/repo").unwrap();
        // The contract requires canonical workspace roots (as jj reports).
        // Canonicalize, then simplify: verbatim (`\\?\…`) on Windows, plain
        // elsewhere — matching what state_dir returns.
        let main =
            dunce::simplified(&parent.path().join("main").canonicalize().unwrap()).to_path_buf();
        let ws2 =
            dunce::simplified(&parent.path().join("ws2").canonicalize().unwrap()).to_path_buf();

        let state = state_dir(&ws2).unwrap();

        assert_eq!(state, main.join(".jj/tatami"));
    }

    #[test]
    fn fails_when_there_is_no_repo_storage_at_all() {
        let root = tempfile::tempdir().unwrap();

        let error = state_dir(root.path()).unwrap_err();

        assert!(
            matches!(error, StateError::NoRepoStorage { .. }),
            "expected NoRepoStorage, got: {error:?}"
        );
    }

    #[test]
    fn fails_when_the_pointer_leads_to_removed_storage() {
        let parent = tempfile::tempdir().unwrap();
        let ws2 = parent.path().join("ws2");
        mkdir_all(&ws2.join(".jj"));
        fs::write(ws2.join(".jj/repo"), "../../gone/.jj/repo").unwrap();

        let error = state_dir(&ws2).unwrap_err();

        assert!(
            matches!(error, StateError::DanglingPointer { .. }),
            "expected DanglingPointer, got: {error:?}"
        );
    }
}

#[cfg(test)]
mod hosts_storage_should {
    use super::*;

    fn mkdir_all(path: &Path) {
        fs::create_dir_all(path).unwrap();
    }

    #[test]
    fn recognizes_the_hosting_workspace_by_its_real_repo_dir() {
        let parent = tempfile::tempdir().unwrap();
        let main = parent.path().join("main");
        let ws2 = parent.path().join("ws2");
        mkdir_all(&main.join(".jj/repo"));
        mkdir_all(&ws2.join(".jj"));
        fs::write(ws2.join(".jj/repo"), "../main/.jj/repo").unwrap();

        assert!(hosts_storage(&main));
        assert!(!hosts_storage(&ws2));
    }

    #[test]
    fn answers_false_outside_any_workspace() {
        let empty = tempfile::tempdir().unwrap();
        assert!(!hosts_storage(empty.path()));
    }
}

#[cfg(test)]
mod file_state_should {
    use super::*;
    use pretty_assertions::assert_eq;

    fn mkdir_all(path: &Path) {
        fs::create_dir_all(path).unwrap();
    }

    /// A workspace-shaped temp dir: hosts its own repo storage, so the state
    /// dir is `<root>/.jj/tatami`.
    fn workspace() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        mkdir_all(&root.path().join(".jj/repo"));
        root
    }

    #[test]
    fn round_trips_the_previous_workspace() {
        let root = workspace();
        let state = FileState::for_workspace(root.path()).unwrap();

        state.set_previous_workspace(Some("ws2")).unwrap();

        assert_eq!(state.previous_workspace().unwrap(), Some("ws2".to_string()));
    }

    #[test]
    fn answers_none_when_nothing_was_recorded() {
        let root = workspace();
        let state = FileState::for_workspace(root.path()).unwrap();

        assert_eq!(state.previous_workspace().unwrap(), None);
    }

    #[test]
    fn clears_the_previous_workspace() {
        let root = workspace();
        let state = FileState::for_workspace(root.path()).unwrap();
        state.set_previous_workspace(Some("ws2")).unwrap();

        state.set_previous_workspace(None).unwrap();

        assert_eq!(state.previous_workspace().unwrap(), None);
    }

    #[test]
    fn stores_the_state_in_the_host_workspaces_tatami_dir() {
        let root = workspace();
        let state = FileState::for_workspace(root.path()).unwrap();

        state.set_previous_workspace(Some("ws2")).unwrap();

        assert_eq!(
            fs::read_to_string(root.path().join(".jj/tatami/previous_workspace")).unwrap(),
            "ws2\n"
        );
    }

    #[test]
    fn treats_an_empty_state_file_as_unrecorded() {
        let root = workspace();
        let state = FileState::for_workspace(root.path()).unwrap();
        fs::create_dir_all(root.path().join(".jj/tatami")).unwrap();
        fs::write(root.path().join(".jj/tatami/previous_workspace"), "  \n").unwrap();

        assert_eq!(state.previous_workspace().unwrap(), None);
    }
}

#[cfg(test)]
mod trash_should {
    use super::*;
    use pretty_assertions::assert_eq;

    fn mkdir_all(path: &Path) {
        fs::create_dir_all(path).unwrap();
    }

    #[test]
    fn lists_trash_entries_sorted_and_tolerates_an_absent_directory() {
        let state = tempfile::tempdir().unwrap();
        assert_eq!(trash_entries(state.path()).unwrap(), Vec::<PathBuf>::new());

        let trash = state.path().join("trash");
        mkdir_all(&trash.join("b"));
        fs::write(trash.join("b/.env"), "SECRET=1").unwrap();
        mkdir_all(&trash.join("a"));

        let entries = trash_entries(state.path()).unwrap();
        assert_eq!(
            entries,
            vec![trash.join("a"), trash.join("b")],
            "oldest name first"
        );
    }

    #[test]
    fn clear_trash_removes_every_entry_and_tolerates_an_absent_directory() {
        let state = tempfile::tempdir().unwrap();
        clear_trash(state.path()).unwrap();

        mkdir_all(&state.path().join("trash/ws1"));
        fs::write(state.path().join("trash/ws1/.env"), "SECRET=1").unwrap();

        clear_trash(state.path()).unwrap();

        assert_eq!(trash_entries(state.path()).unwrap(), Vec::<PathBuf>::new());
    }
}
