//! `tt switch --create <nom>` — create a workspace (and its namesake
//! bookmark), then behave like a switch to it.
//!
//! Destination rules (worktrunk parity, sourced from its docs):
//! - no template configured → a **sibling of the repo root** named
//!   `<repo>.<workspace>` (`../<repo>.<branch>` upstream);
//! - `workspace-path` (legacy `worktree-path`) configured → expanded with a
//!   minimal variable set; relative templates resolve against the current
//!   workspace root, `~` against the home directory.
//!   shortcut: hand-rolled expansion for `{{ workspace }}`/`{{ branch }}`
//!   and the `sanitize` filter only — the full minijinja engine (all
//!   variables/filters) lands in M4, when aliases and hooks need it too.
//!
//! jj auto-excludes nested workspaces from the surrounding snapshot
//! (verified empirically on 0.47.1), so an in-repo template like
//! `.wt/{{ workspace }}` is safe.

use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::jj::{Jj, JjError};

pub fn run(
    jj: &dyn Jj,
    name: &str,
    template: Option<&str>,
    current_root: &Path,
    home: &Path,
) -> Result<PathBuf, CreateError> {
    validate_name(name)?;
    let destination = resolve_destination(template, name, current_root, home)?;
    if let Some(parent) = destination.parent() {
        // jj requires the destination's parent to exist (verified 0.47.1:
        // `Cannot access … No such file or directory` otherwise).
        fs::create_dir_all(parent).map_err(|source| CreateError::Io { source })?;
    }
    jj.workspace_add(name, &destination)?;
    jj.bookmark_create(name, name)?;
    Ok(destination)
}

/// Workspace names become bookmark names, so both must accept them: letters,
/// digits, `-`, `_`, `/`, `.` — but no leading `-` (flag collision) or `.`.
fn validate_name(name: &str) -> Result<(), CreateError> {
    let valid = !name.is_empty()
        && !name.starts_with(['-', '.'])
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '/' | '.'));
    if valid {
        Ok(())
    } else {
        Err(CreateError::InvalidName {
            name: name.to_string(),
        })
    }
}

/// `workspace-path` minus the worktrunk filters tatami does not implement
/// yet: anything outside `[A-Za-z0-9._-]` becomes `-`.
fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn resolve_destination(
    template: Option<&str>,
    name: &str,
    current_root: &Path,
    home: &Path,
) -> Result<PathBuf, CreateError> {
    let raw = match template {
        Some(template) => {
            let expanded = template
                .replace("{{ workspace | sanitize }}", &sanitize(name))
                .replace("{{ workspace }}", name)
                .replace("{{ branch | sanitize }}", &sanitize(name))
                .replace("{{ branch }}", name);
            match expanded.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => PathBuf::from(expanded),
            }
        }
        None => {
            let repo_name =
                current_root
                    .file_name()
                    .ok_or_else(|| CreateError::UnresolvableDestination {
                        root: current_root.to_path_buf(),
                    })?;
            let sibling =
                current_root
                    .parent()
                    .ok_or_else(|| CreateError::UnresolvableDestination {
                        root: current_root.to_path_buf(),
                    })?;
            sibling.join(format!("{}.{name}", repo_name.to_string_lossy()))
        }
    };

    // Relative templates resolve against the current workspace root, like
    // worktrunk resolves its relative `worktree-path`.
    if raw.is_absolute() {
        Ok(raw)
    } else {
        Ok(current_root.join(raw))
    }
}

#[derive(Debug, Error)]
pub enum CreateError {
    #[error(
        "invalid workspace name `{name}`: use letters, digits, `-`, `_`, `/`, `.` (no leading `-` or `.`)"
    )]
    InvalidName { name: String },
    #[error("cannot resolve a destination from root {root}")]
    UnresolvableDestination { root: PathBuf },
    #[error("filesystem error: {source}")]
    Io {
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Jj(#[from] JjError),
}

#[cfg(test)]
mod resolve_destination_should {
    use super::*;
    use pretty_assertions::assert_eq;

    fn root() -> PathBuf {
        PathBuf::from("/repo/main")
    }

    #[test]
    fn defaults_to_a_sibling_of_the_repo_root() {
        let dest = resolve_destination(None, "feature", &root(), Path::new("/home/u")).unwrap();
        assert_eq!(dest, PathBuf::from("/repo/main.feature"));
    }

    #[test]
    fn expands_the_workspace_variable_relative_to_the_current_root() {
        let dest = resolve_destination(
            Some(".wt/{{ workspace }}"),
            "feature",
            &root(),
            Path::new("/home/u"),
        )
        .unwrap();
        assert_eq!(dest, PathBuf::from("/repo/main/.wt/feature"));
    }

    #[test]
    fn sanitizes_unsafe_characters_for_the_filesystem() {
        let dest = resolve_destination(
            Some(".wt/{{ workspace | sanitize }}"),
            "feature/login-fix",
            &root(),
            Path::new("/home/u"),
        )
        .unwrap();
        assert_eq!(dest, PathBuf::from("/repo/main/.wt/feature-login-fix"));
    }

    #[test]
    fn honors_the_legacy_branch_variable_as_an_alias() {
        let dest = resolve_destination(
            Some(".wt/{{ branch }}"),
            "feature",
            &root(),
            Path::new("/home/u"),
        )
        .unwrap();
        assert_eq!(dest, PathBuf::from("/repo/main/.wt/feature"));
    }

    #[test]
    fn keeps_absolute_templates_absolute() {
        let dest = resolve_destination(
            Some("/elsewhere/{{ workspace }}"),
            "feature",
            &root(),
            Path::new("/home/u"),
        )
        .unwrap();
        assert_eq!(dest, PathBuf::from("/elsewhere/feature"));
    }

    #[test]
    fn expands_a_leading_tilde_against_the_home_directory() {
        let dest = resolve_destination(
            Some("~/workspaces/{{ workspace }}"),
            "feature",
            &root(),
            Path::new("/home/u"),
        )
        .unwrap();
        assert_eq!(dest, PathBuf::from("/home/u/workspaces/feature"));
    }

    #[test]
    fn leaves_unknown_variables_as_literals_for_now() {
        let dest = resolve_destination(
            Some(".wt/{{ owner }}-{{ workspace }}"),
            "feature",
            &root(),
            Path::new("/home/u"),
        )
        .unwrap();
        assert_eq!(dest, PathBuf::from("/repo/main/.wt/{{ owner }}-feature"));
    }

    #[test]
    fn rejects_names_that_cannot_be_bookmarks() {
        for bad in ["", "-flag", ".hidden", "a b", "a:b"] {
            let error = validate_name(bad).unwrap_err();
            assert!(
                matches!(error, CreateError::InvalidName { .. }),
                "name {bad:?} should be rejected"
            );
        }
        for good in ["feature", "feature/x", "fix_2026", "a.b"] {
            validate_name(good).expect("name should be accepted");
        }
    }
}
