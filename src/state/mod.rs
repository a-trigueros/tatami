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

/// Absolute directory where tatami's repo-global state lives for the repo
/// reachable from `workspace_root`.
///
/// `workspace_root` must be an existing, canonical workspace root — the shape
/// jj itself reports (`jj workspace root` resolves symlinks). Non-canonical
/// inputs would leak symlink differences into the output.
pub fn state_dir(workspace_root: &Path) -> Result<PathBuf, StateError> {
    let repo_probe = workspace_root.join(".jj").join("repo");
    if repo_probe.is_dir() {
        // This workspace hosts the shared storage.
        return Ok(workspace_root.join(".jj").join("tatami"));
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
    let host_root = storage
        .parent()
        .and_then(|jj_dir| jj_dir.parent())
        .ok_or_else(|| StateError::MalformedPointer {
            path: repo_probe.clone(),
        })?;
    Ok(host_root.join(".jj").join("tatami"))
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
        let main = parent.path().join("main").canonicalize().unwrap();
        let ws2 = parent.path().join("ws2").canonicalize().unwrap();

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
