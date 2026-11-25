//! `tt switch --create <nom>` — create a workspace (and its namesake
//! bookmark), then behave like a switch to it.
//!
//! Destination rules (worktrunk parity, sourced from its docs):
//! - no template configured → a **sibling of the root repository** named
//!   `<repo>.<workspace>` (`../<repo>.<branch>` upstream);
//! - `workspace-path` (legacy `worktree-path`) configured → expanded with a
//!   minimal variable set; `{{ repo }}` is the **root repository's** directory
//!   name (the workspace hosting the shared jj storage), not the current
//!   workspace's (issue #1); relative templates resolve against the current
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

use crate::hook::{self, ApprovalGate, HookContext, HookError, HookParts, HookPhase};
use crate::jj::{Jj, JjError};
use crate::state::StateError;

pub fn run(
    jj: &dyn Jj,
    name: &str,
    template: Option<&str>,
    current_root: &Path,
    home: &Path,
    parts: HookParts<'_>,
    gate: &mut dyn ApprovalGate,
) -> Result<PathBuf, CreateError> {
    validate_name(name)?;
    // Repo identity anchors on the root repository — the workspace hosting
    // the shared jj storage. Creating from inside a workspace (issue #1)
    // must not adopt the current workspace's name as the repo's.
    let repo_root = crate::state::host_root(current_root)?;
    let destination = resolve_destination(template, name, current_root, &repo_root, home)?;
    if let Some(parent) = destination.parent() {
        // jj requires the destination's parent to exist (verified 0.47.1:
        // `Cannot access … No such file or directory` otherwise).
        fs::create_dir_all(parent).map_err(|source| CreateError::Io { source })?;
    }
    // Pass the destination RELATIVE to jj's cwd (= current_root): on Windows,
    // an absolute destination is verbatim-ized by jj (`\\?\C:\…`) and the
    // internal `git worktree add` then fails to create it (pinned 0.47.1,
    // observed on the runner). Relative form: inside current_root directly,
    // otherwise relative to its parent (the sibling default layout).
    let jj_destination = destination
        .strip_prefix(current_root)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| {
            destination
                .strip_prefix(current_root.parent().unwrap_or(current_root))
                .map(|p| PathBuf::from("..").join(p))
                .unwrap_or_else(|_| destination.clone())
        });
    crate::cmd::with_stale_recovery(jj, |jj| jj.workspace_add(name, &jj_destination))?;
    jj.bookmark_create(name, name)?;

    // Worktrunk-sourced order: creation → pre-start (blocking) → post-start
    // (detached). A failed pre-start rolls the fresh creation back — a
    // half-created workspace must not survive.
    let hook_ctx = |phase| {
        Some(HookContext {
            phase,
            workspace: name,
            workspace_path: &destination,
            hooks: parts.hooks,
            logs_dir: parts.logs_dir,
        })
    };
    if let Some(ctx) = hook_ctx(HookPhase::PreStart)
        && let Err(error) = hook::run_blocking(&ctx, gate)
    {
        let _ = jj.workspace_forget(name);
        if destination.exists() {
            let _ = fs::remove_dir_all(&destination);
        }
        return Err(error.into());
    }
    if let Some(ctx) = hook_ctx(HookPhase::PostStart) {
        // No writer here: detached-hook warnings surface on the caller's
        // stderr in the switch flow; standalone creation stays quiet.
        hook::run_detached_quiet(&ctx, gate);
    }
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

fn resolve_destination(
    template: Option<&str>,
    name: &str,
    current_root: &Path,
    repo_root: &Path,
    home: &Path,
) -> Result<PathBuf, CreateError> {
    let raw = match template {
        Some(template) => {
            let mut vars = crate::template::TemplateVars::new();
            vars.insert("workspace", name.to_string())
                .insert("branch", name.to_string());
            // Worktrunk parity: `repo` is the repository directory name —
            // the root repository's, not the current workspace's (issue #1:
            // creating from inside a workspace keeps the repo's identity).
            if let Some(repo) = repo_root.file_name() {
                vars.insert("repo", repo.to_string_lossy().to_string());
            }
            // Strict: a wrong path is a real hazard — `~/factory/{{ repo }}`
            // must fail loudly, not build `~/factory//name`.
            let expanded = crate::template::render_strict(template, &vars)?;
            match expanded.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => PathBuf::from(expanded),
            }
        }
        None => {
            // Worktrunk's default layout sanitizes the name: the workspace
            // `feat/test` is one directory `repo.feat-test`, not a nested
            // `repo.feat/test`. The anchor is the root repository, not the
            // current workspace (issue #1).
            let repo_name =
                repo_root
                    .file_name()
                    .ok_or_else(|| CreateError::UnresolvableDestination {
                        root: repo_root.to_path_buf(),
                    })?;
            let sibling =
                repo_root
                    .parent()
                    .ok_or_else(|| CreateError::UnresolvableDestination {
                        root: repo_root.to_path_buf(),
                    })?;
            sibling.join(format!(
                "{}.{}",
                repo_name.to_string_lossy(),
                crate::template::sanitize(name)
            ))
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
    #[error(transparent)]
    State(#[from] StateError),
    #[error("filesystem error: {source}")]
    Io {
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Hook(#[from] HookError),
    #[error(transparent)]
    Template(#[from] crate::template::TemplateError),
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
        let dest =
            resolve_destination(None, "feature", &root(), &root(), Path::new("/home/u")).unwrap();
        assert_eq!(dest, PathBuf::from("/repo/main.feature"));
    }

    #[test]
    fn defaults_to_a_sibling_of_the_root_repository_even_from_a_secondary_workspace() {
        // Issue #1: the sibling rule anchors on the repo root — creating from
        // inside a workspace must not nest the new one under the current one.
        let current_workspace = Path::new("/repo/factory/myrepo/feat-myfeat1");
        let dest = resolve_destination(
            None,
            "fix/emergency",
            current_workspace,
            Path::new("/repo/myrepo"),
            Path::new("/home/u"),
        )
        .unwrap();
        assert_eq!(dest, PathBuf::from("/repo/myrepo.fix-emergency"));
    }

    #[test]
    fn expands_the_workspace_variable_relative_to_the_current_root() {
        let dest = resolve_destination(
            Some(".wt/{{ workspace }}"),
            "feature",
            &root(),
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
            &root(),
            Path::new("/home/u"),
        )
        .unwrap();
        assert_eq!(dest, PathBuf::from("/home/u/workspaces/feature"));
    }

    #[test]
    fn unknown_variables_fail_the_path_template_instead_of_building_a_wrong_path() {
        // Strict mode: `{{ owner }}` (worktrunk's variable, not provided by
        // tatami) must fail loudly — a silent empty segment once built
        // `~/factory//name` from `~/factory/{{ repo }}/…`.
        let error = resolve_destination(
            Some(".wt/{{ owner }}-{{ workspace }}"),
            "feature",
            &root(),
            &root(),
            Path::new("/home/u"),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("undefined"),
            "expected an undefined-variable error: {error}"
        );
    }

    #[test]
    fn provides_repo_as_the_repository_directory_name() {
        // Worktrunk-parity variable: `~/factory/{{ repo }}/{{ workspace }}`.
        let dest = resolve_destination(
            Some("~/factory/{{ repo }}/{{ workspace }}"),
            "feature",
            &root(),
            &root(),
            Path::new("/home/u"),
        )
        .unwrap();
        assert_eq!(dest, PathBuf::from("/home/u/factory/main/feature"));
    }

    #[test]
    fn resolves_the_repo_variable_from_the_root_repository_not_the_current_workspace() {
        // Issue #1: running `tt switch -c` from inside a secondary workspace
        // (`<repo>/feat-myfeat1`) must still build the path from the repo's
        // name — the current workspace's name is not the repository's.
        let current_workspace = Path::new("/repo/factory/myrepo/feat-myfeat1");
        let dest = resolve_destination(
            Some("~/factory/{{ repo }}/{{ branch | sanitize }}"),
            "fix/emergency",
            current_workspace,
            Path::new("/repo/myrepo"),
            Path::new("/home/u"),
        )
        .unwrap();
        assert_eq!(dest, PathBuf::from("/home/u/factory/myrepo/fix-emergency"));
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
