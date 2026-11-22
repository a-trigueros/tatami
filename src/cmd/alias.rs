//! `tt <alias> [args…]` — run an `[aliases]` entry from the user config and
//! (approval-gated) from the project config. Worktrunk-sourced semantics:
//! positional arguments land in `{{ args }}` (space-joined, shell-escaped);
//! `--KEY=VALUE` / `--KEY VALUE` bind template variables; user hooks run
//! first, project hooks are gated; a failing command aborts (exit 1).

use std::path::Path;
use std::process::Command;

use thiserror::Error;

use crate::cmd::with_stale_recovery;
use crate::config::schema::{CommandSpec, ProjectConfig, UserConfig};
use crate::hook::ApprovalGate;
use crate::jj::{Jj, JjError};
use crate::template::{self, TemplateVars};

pub fn run(
    jj: &dyn Jj,
    name: &str,
    raw_args: &[String],
    user: &UserConfig,
    project: &ProjectConfig,
    gate: &mut dyn ApprovalGate,
    caller_root: &Path,
) -> Result<(), AliasError> {
    let user_spec = user.aliases.as_ref().and_then(|a| a.get(name));
    let project_spec = project.aliases.as_ref().and_then(|a| a.get(name));
    if user_spec.is_none() && project_spec.is_none() {
        return Err(AliasError::UnknownAlias {
            name: name.to_string(),
        });
    }

    let workspace = with_stale_recovery(jj, |jj| jj.workspaces())?
        .into_iter()
        .find(|w| w.is_current)
        .map(|w| w.name)
        .ok_or(AliasError::CurrentWorkspaceNotFound)?;

    let vars = alias_vars(workspace, caller_root, raw_args);

    if let Some(spec) = user_spec {
        execute_blocking(spec, caller_root, &vars)?;
    }
    if let Some(spec) = project_spec {
        for (_entry_name, command) in entries(spec) {
            if !gate.approve(name, &command) {
                return Err(AliasError::Denied {
                    alias: name.to_string(),
                    command,
                });
            }
        }
        execute_blocking(spec, caller_root, &vars)?;
    }
    Ok(())
}

/// Positional arguments become `{{ args }}` (shell-escaped, space-joined);
/// `--KEY=VALUE` / `--KEY VALUE` become template variables.
fn alias_vars(workspace: String, caller_root: &Path, raw_args: &[String]) -> TemplateVars {
    let mut vars = TemplateVars::new();
    vars.insert("workspace", workspace)
        .insert("workspace_path", caller_root.display().to_string())
        .insert("repo_path", caller_root.display().to_string())
        .insert("cwd", caller_root.display().to_string());

    let mut positional = Vec::new();
    let mut binding_next: Option<String> = None;
    for raw in raw_args {
        if let Some(key) = binding_next.take() {
            vars.insert(&key, raw.clone());
            continue;
        }
        if raw == "--" {
            // Everything after `--` is positional; tatami keeps scanning so
            // callers do not need to drop the separator themselves.
            continue;
        }
        if let Some(binding) = raw.strip_prefix("--") {
            if let Some((key, value)) = binding.split_once('=') {
                vars.insert(key, value.to_string());
                continue;
            }
            binding_next = Some(binding.to_string());
            continue;
        }
        positional.push(shell_escape(raw));
    }
    vars.insert("args", positional.join(" "));
    vars
}

/// Single-quote wrapping with the POSIX escape for embedded quotes.
fn shell_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn entries(spec: &CommandSpec) -> Vec<(String, String)> {
    match spec {
        CommandSpec::Single(command) => vec![(String::new(), command.clone())],
        CommandSpec::Concurrent(map) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        CommandSpec::Pipeline(steps) => steps
            .iter()
            .flat_map(|step| step.iter().map(|(k, v)| (k.clone(), v.clone())))
            .collect(),
    }
}

/// Fail-fast blocking execution, hook-parity: first failing command stops.
fn execute_blocking(spec: &CommandSpec, cwd: &Path, vars: &TemplateVars) -> Result<(), AliasError> {
    for (_name, command) in entries(spec) {
        let rendered = template::render(&command, vars).map_err(|error| AliasError::Template {
            command: command.clone(),
            source: error,
        })?;
        let status = Command::new("sh")
            .arg("-c")
            .arg(&rendered)
            .current_dir(cwd)
            .status()
            .map_err(|source| AliasError::Spawn {
                command: rendered.clone(),
                source,
            })?;
        if !status.success() {
            return Err(AliasError::Failed {
                command: rendered,
                status: status.code().unwrap_or(1),
            });
        }
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum AliasError {
    #[error(transparent)]
    Jj(#[from] JjError),
    #[error("no alias named `{name}`")]
    UnknownAlias { name: String },
    #[error("cannot determine the current workspace")]
    CurrentWorkspaceNotFound,
    #[error("project alias `{alias}` command `{command}` was not approved")]
    Denied { alias: String, command: String },
    #[error("alias command `{command}` could not start: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("alias command `{command}` failed with exit code {status}")]
    Failed { command: String, status: i32 },
    #[error("alias template `{command}` could not be rendered: {source}")]
    Template {
        command: String,
        #[source]
        source: crate::template::TemplateError,
    },
}
