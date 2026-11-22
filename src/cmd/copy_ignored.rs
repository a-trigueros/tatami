//! `tt step copy-ignored [--from <ws>] [--to <ws>] [--force]` — copy
//! git-ignored files from one workspace to another.
//!
//! Worktrunk-sourced semantics: only **ignored** files move (tracked and
//! merely-untracked files are not touched). Ignore rules = the root
//! `.gitignore`, `.git/info/exclude` (when colocated), and per-directory
//! nested `.gitignore` files — deepest match wins, `!pattern` whitelists.
//! Must work in pure-jj repos without a `.git` directory. `.worktreeinclude`
//! in the source filters the selection gitignore-style when present.
//! Existing destination files are skipped unless `--force`.
//! shortcut: plain copies, no reflink; global git excludes not honored.

use std::fs;
use std::path::{Path, PathBuf};

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use thiserror::Error;

use crate::cmd::with_stale_recovery;
use crate::jj::{Jj, JjError};

/// Paths tatami never copies, whatever the ignore rules say.
const NEVER_COPY: [&str; 2] = [".jj", ".git"];

pub struct CopyIgnoredArgs<'a> {
    /// Source workspace; defaults to the primary (host) workspace.
    pub from: Option<&'a str>,
    /// Destination workspace; defaults to the current one.
    pub to: Option<&'a str>,
    /// Overwrite destination files that already exist.
    pub force: bool,
}

#[derive(Debug)]
pub struct CopyReport {
    pub copied: Vec<PathBuf>,
    pub skipped_existing: Vec<PathBuf>,
}

pub fn run(jj: &dyn Jj, args: CopyIgnoredArgs<'_>) -> Result<CopyReport, CopyError> {
    let workspaces = with_stale_recovery(jj, |jj| jj.workspaces())?;
    let current = workspaces
        .iter()
        .find(|w| w.is_current)
        .ok_or(CopyError::CurrentWorkspaceNotFound)?;
    let host_root = crate::state::host_root(
        current
            .root
            .as_deref()
            .ok_or(CopyError::CurrentWorkspaceNotFound)?,
    )?;

    let from = resolve_workspace(&workspaces, args.from, |w| {
        w.root.as_deref() == Some(host_root.as_path())
    })?;
    let to = resolve_workspace(&workspaces, args.to, |w| w.is_current)?;

    let (from_root, to_root) = match (&from.root, &to.root) {
        (Some(from_root), Some(to_root)) => (from_root.clone(), to_root.clone()),
        _ => return Err(CopyError::RootNotRecorded),
    };
    if !from_root.is_dir() {
        return Err(CopyError::SourceDirectoryGone {
            workspace: from.name.clone(),
            path: from_root.clone(),
        });
    }

    let ignored = collect_ignored(&from_root)?;
    let include = worktreeinclude_matcher(&from_root)?;

    let mut report = CopyReport {
        copied: Vec::new(),
        skipped_existing: Vec::new(),
    };
    for relative in ignored {
        if let Some(matcher) = &include
            && matches!(matcher.matched(&relative, false), Match::None)
        {
            continue;
        }
        let destination_path = to_root.join(&relative);
        if destination_path.exists() && !args.force {
            report.skipped_existing.push(destination_path);
            continue;
        }
        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent).map_err(|source| CopyError::Io { source })?;
        }
        fs::copy(from_root.join(&relative), &destination_path)
            .map_err(|source| CopyError::Io { source })?;
        report.copied.push(destination_path);
    }
    Ok(report)
}

fn resolve_workspace<'w>(
    workspaces: &'w [crate::jj::WorkspaceInfo],
    name: Option<&str>,
    default_predicate: impl Fn(&crate::jj::WorkspaceInfo) -> bool,
) -> Result<&'w crate::jj::WorkspaceInfo, CopyError> {
    match name {
        Some(explicit) => workspaces
            .iter()
            .find(|w| w.name == explicit)
            .ok_or_else(|| CopyError::UnknownWorkspace {
                name: explicit.to_string(),
            }),
        None => {
            workspaces
                .iter()
                .find(|w| default_predicate(w))
                .ok_or(CopyError::NoDefaultWorkspace {
                    available: workspaces
                        .iter()
                        .map(|w| w.name.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                })
        }
    }
}

/// Ignore rules for one directory: its `.gitignore` (deepest match wins over
/// shallower directories' rules, git semantics) plus the inherited lines.
struct DirRules {
    matcher: Gitignore,
    lines: Vec<String>,
}

/// Collects the ignored files under `root`. The rule stack is carried
/// explicitly through the recursion (deepest first on lookup).
fn collect_ignored(root: &Path) -> Result<Vec<PathBuf>, CopyError> {
    let mut stack = vec![rules_for(root, &[])?];
    let mut ignored = Vec::new();
    walk(root, root, &mut stack, &mut ignored)?;
    Ok(ignored)
}

fn rules_for(directory: &Path, parent_stack: &[DirRules]) -> Result<DirRules, CopyError> {
    let mut builder = GitignoreBuilder::new(directory);
    // Root-only: the colocated `.git/info/exclude` carries personal excludes.
    let exclude = directory.join(".git").join("info").join("exclude");
    if exclude.is_file() {
        builder.add(&exclude);
    }
    // Inherited lines first (they lose ties against the directory's own
    // patterns, matching git's deeper-file-wins rule), own lines last.
    let mut lines = Vec::new();
    for parent in parent_stack {
        for line in &parent.lines {
            builder
                .add_line(None, line)
                .map_err(|error| CopyError::IgnoreRules {
                    directory: directory.to_path_buf(),
                    reason: error.to_string(),
                })?;
            lines.push(line.clone());
        }
    }
    let gitignore = directory.join(".gitignore");
    if gitignore.is_file() {
        let raw = fs::read_to_string(&gitignore).map_err(|source| CopyError::Io { source })?;
        for line in raw.lines() {
            builder
                .add_line(None, line)
                .map_err(|error| CopyError::IgnoreRules {
                    directory: directory.to_path_buf(),
                    reason: error.to_string(),
                })?;
            lines.push(line.to_string());
        }
    }
    Ok(DirRules {
        matcher: builder.build().map_err(|error| CopyError::IgnoreRules {
            directory: directory.to_path_buf(),
            reason: error.to_string(),
        })?,
        lines,
    })
}

/// Is `path` ignored under `stack`? Deepest rules win; `!pattern` whitelists.
fn is_ignored(stack: &[DirRules], path: &Path, is_dir: bool) -> bool {
    for rules in stack.iter().rev() {
        match rules.matcher.matched(path, is_dir) {
            Match::Ignore(_) => return true,
            Match::Whitelist(_) => return false,
            Match::None => {}
        }
    }
    false
}

fn walk(
    directory: &Path,
    root: &Path,
    stack: &mut Vec<DirRules>,
    ignored: &mut Vec<PathBuf>,
) -> Result<(), CopyError> {
    let entries = fs::read_dir(directory).map_err(|source| CopyError::Io { source })?;
    for entry in entries {
        let path = entry.map_err(|source| CopyError::Io { source })?.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if NEVER_COPY.contains(&file_name) {
            continue;
        }
        if is_ignored(stack, &path, path.is_dir()) {
            if path.is_dir() {
                collect_whole_directory(&path, root, ignored)?;
            } else {
                ignored.push(path.strip_prefix(root).expect("under root").to_path_buf());
            }
            continue;
        }
        if path.is_dir() {
            stack.push(rules_for(&path, stack)?);
            walk(&path, root, stack, ignored)?;
            stack.pop();
        }
    }
    Ok(())
}

fn collect_whole_directory(
    directory: &Path,
    root: &Path,
    ignored: &mut Vec<PathBuf>,
) -> Result<(), CopyError> {
    let entries = fs::read_dir(directory).map_err(|source| CopyError::Io { source })?;
    for entry in entries {
        let path = entry.map_err(|source| CopyError::Io { source })?.path();
        if path.is_dir() {
            collect_whole_directory(&path, root, ignored)?;
        } else {
            ignored.push(path.strip_prefix(root).expect("under root").to_path_buf());
        }
    }
    Ok(())
}

/// Gitignore-style matcher over the source `.worktreeinclude`, when present.
fn worktreeinclude_matcher(root: &Path) -> Result<Option<Gitignore>, CopyError> {
    let file = root.join(".worktreeinclude");
    if !file.is_file() {
        return Ok(None);
    }
    let mut builder = GitignoreBuilder::new(root);
    builder.add(&file);
    let gitignore = builder.build().map_err(|error| CopyError::IgnoreRules {
        directory: file.parent().unwrap_or(root).to_path_buf(),
        reason: error.to_string(),
    })?;
    Ok(Some(gitignore))
}

#[derive(Debug, Error)]
pub enum CopyError {
    #[error(transparent)]
    Jj(#[from] JjError),
    #[error(transparent)]
    State(#[from] crate::state::StateError),
    #[error("cannot determine the current workspace")]
    CurrentWorkspaceNotFound,
    #[error("no workspace named `{name}`")]
    UnknownWorkspace { name: String },
    #[error("no default source workspace: the host workspace must exist (available: {available})")]
    NoDefaultWorkspace { available: String },
    #[error("workspace root not recorded (jj < 0.38)")]
    RootNotRecorded,
    #[error("source workspace `{workspace}` directory is gone ({path})")]
    SourceDirectoryGone { workspace: String, path: PathBuf },
    #[error("filesystem error: {source}")]
    Io {
        #[source]
        source: std::io::Error,
    },
    #[error("cannot parse ignore rules in {directory}: {reason}")]
    IgnoreRules { directory: PathBuf, reason: String },
}
