//! Hook execution over the user config's lifecycle hooks (PLAN §4 shapes).
//!
//! Two execution modes:
//! - **blocking** (`pre-*`): entries run sequentially; the first failure
//!   aborts the caller's operation. shortcut: entries within a "concurrent"
//!   step also run sequentially — true concurrency is only required for
//!   long-running processes, which are `post-*` territory.
//! - **detached** (`post-*`): commands spawn detached and never block, so a
//!   `post-start = { server = "npm run dev" }` does not hang the switch.
//!   Detached processes have no failure detection; their output goes to the
//!   caller's terminal. shortcut: log-file routing lands with M4 polish.
//!
//! Template variables today: `{{ workspace }}`, `{{ workspace_path }}`.
//! shortcut: the full minijinja variable/filter set lands in M4.
//!
//! Hooks come from the **user config only** for now; the approvals system is
//! meaningful once hooks can come from a shared project config
//! (`.config/tatami.toml` → `wt.toml`), which is the next slice.

use std::io::Write;
use std::path::Path;
use std::process::Command;

use thiserror::Error;

use crate::config::schema::{CommandSpec, UserConfig};

/// The lifecycle phases tatami executes today. `pre-commit`/`pre-merge` etc.
/// arrive with their commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookPhase {
    PreSwitch,
    PostSwitch,
    PreStart,
    PostStart,
    PreRemove,
    PostRemove,
}

impl HookPhase {
    /// The kebab-case config key, for error messages.
    pub fn config_key(self) -> &'static str {
        match self {
            Self::PreSwitch => "pre-switch",
            Self::PostSwitch => "post-switch",
            Self::PreStart => "pre-start",
            Self::PostStart => "post-start",
            Self::PreRemove => "pre-remove",
            Self::PostRemove => "post-remove",
        }
    }
}

/// Everything a hook template can render and run against.
pub struct HookContext<'a> {
    pub phase: HookPhase,
    pub workspace: &'a str,
    pub workspace_path: &'a Path,
    pub config: &'a UserConfig,
}

impl HookContext<'_> {
    fn render(&self, command: &str) -> String {
        command.replace("{{ workspace }}", self.workspace).replace(
            "{{ workspace_path }}",
            &self.workspace_path.display().to_string(),
        )
    }

    fn spec(&self) -> Option<&CommandSpec> {
        self.config.hook(self.phase)
    }
}

#[derive(Debug, Error)]
pub enum HookError {
    /// A blocking hook failed — the caller's operation must abort.
    #[error("`{phase}` hook command `{command}` failed with {status}")]
    Failed {
        phase: &'static str,
        command: String,
        status: std::process::ExitStatus,
    },
    #[error("`{phase}` hook command `{command}` could not start: {source}")]
    Spawn {
        phase: &'static str,
        command: String,
        #[source]
        source: std::io::Error,
    },
}

/// Runs a `pre-*` phase: the first failing command aborts.
pub fn run_blocking(ctx: &HookContext<'_>) -> Result<(), HookError> {
    let Some(spec) = ctx.spec() else {
        return Ok(());
    };
    execute_blocking(spec, ctx, ctx.workspace_path)
}

/// Runs a `post-*` phase: commands spawn detached, failures are reported on
/// `out` without failing the caller's operation.
pub fn run_detached(ctx: &HookContext<'_>, out: &mut dyn Write) {
    let Some(spec) = ctx.spec() else {
        return;
    };
    if let Err(error) = execute_detached(spec, ctx, ctx.workspace_path) {
        let _ = writeln!(out, "tatami: warning: {}", error);
    }
}

/// [`run_detached`] for flows without a warning channel (standalone
/// creation): warnings are discarded.
pub fn run_detached_quiet(ctx: &HookContext<'_>) {
    let mut sink = std::io::sink();
    run_detached(ctx, &mut sink);
}

fn entries(spec: &CommandSpec) -> Vec<(String, String)> {
    // (name, command) in shape order: single, concurrent group, pipeline
    // steps flattened in order.
    match spec {
        CommandSpec::Single(command) => vec![(String::new(), command.clone())],
        CommandSpec::Concurrent(map) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        CommandSpec::Pipeline(steps) => steps
            .iter()
            .flat_map(|step| step.iter().map(|(k, v)| (k.clone(), v.clone())))
            .collect(),
    }
}

fn execute_blocking(
    spec: &CommandSpec,
    ctx: &HookContext<'_>,
    cwd: &Path,
) -> Result<(), HookError> {
    for (_name, command) in entries(spec) {
        // Names label commands in worktrunk; tatami reports the command itself.
        let rendered = ctx.render(&command);
        let status = Command::new("sh")
            .arg("-c")
            .arg(&rendered)
            .current_dir(cwd)
            .status()
            .map_err(|source| HookError::Spawn {
                phase: ctx.phase.config_key(),
                command: rendered.clone(),
                source,
            })?;
        if !status.success() {
            return Err(HookError::Failed {
                phase: ctx.phase.config_key(),
                command: rendered,
                status,
            });
        }
    }
    Ok(())
}

fn execute_detached(
    spec: &CommandSpec,
    ctx: &HookContext<'_>,
    cwd: &Path,
) -> Result<(), HookError> {
    for (_name, command) in entries(spec) {
        let rendered = ctx.render(&command);
        Command::new("sh")
            .arg("-c")
            .arg(&rendered)
            .current_dir(cwd)
            .spawn()
            .map_err(|source| HookError::Spawn {
                phase: ctx.phase.config_key(),
                command: rendered,
                source,
            })?;
    }
    Ok(())
}

#[cfg(test)]
mod hook_should {
    use super::*;
    use crate::config::schema::UserConfig;
    use std::path::PathBuf;

    fn config(raw: &str) -> UserConfig {
        toml::from_str(raw).unwrap()
    }

    fn ctx<'a>(
        phase: HookPhase,
        config: &'a UserConfig,
        workspace: &'a str,
        workspace_path: &'a Path,
    ) -> HookContext<'a> {
        HookContext {
            phase,
            workspace,
            workspace_path,
            config,
        }
    }

    fn scratch() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        (dir, path)
    }

    #[test]
    fn runs_a_single_command_in_the_workspace_cwd() {
        let (_dir, cwd) = scratch();
        let cfg = config("pre-start = \"echo started > marker.txt\"");
        let ctx = ctx(HookPhase::PreStart, &cfg, "ws2", &cwd);

        run_blocking(&ctx).unwrap();

        assert_eq!(
            std::fs::read_to_string(cwd.join("marker.txt")).unwrap(),
            "started\n"
        );
    }

    #[test]
    fn expands_the_workspace_variables() {
        let (_dir, cwd) = scratch();
        let cfg = config("pre-start = \"echo {{ workspace }} at {{ workspace_path }} > out.txt\"");
        let ctx = ctx(HookPhase::PreStart, &cfg, "feature", &cwd);

        run_blocking(&ctx).unwrap();

        let rendered = std::fs::read_to_string(cwd.join("out.txt")).unwrap();
        let expected = cwd.display().to_string();
        assert!(
            rendered.contains("feature") && rendered.contains(expected.as_str()),
            "rendered was: {rendered:?}"
        );
    }

    #[test]
    fn runs_every_entry_of_a_concurrent_step_blocking() {
        let (_dir, cwd) = scratch();
        let cfg = config(
            r#"
            [pre-start]
            one = "echo 1 > one.txt"
            two = "echo 2 > two.txt"
            "#,
        );
        let ctx = ctx(HookPhase::PreStart, &cfg, "ws", &cwd);

        run_blocking(&ctx).unwrap();

        assert!(cwd.join("one.txt").exists());
        assert!(cwd.join("two.txt").exists());
    }

    #[test]
    fn stops_a_pipeline_at_the_first_failing_entry() {
        let (_dir, cwd) = scratch();
        let cfg = config(
            r#"
            [[pre-start]]
            broken = "exit 3"

            [[pre-start]]
            never = "echo unreachable > never.txt"
            "#,
        );
        let ctx = ctx(HookPhase::PreStart, &cfg, "ws", &cwd);

        let error = run_blocking(&ctx).unwrap_err();

        match error {
            HookError::Failed { phase, command, .. } => {
                assert_eq!(phase, "pre-start");
                assert_eq!(command, "exit 3");
            }
            other => panic!("expected Failed, got: {other:?}"),
        }
        assert!(!cwd.join("never.txt").exists());
    }

    #[test]
    fn detaches_post_commands_without_blocking() {
        let (_dir, cwd) = scratch();
        let cfg = config("post-start = \"echo detached > post.txt\"");
        let ctx = ctx(HookPhase::PostStart, &cfg, "ws", &cwd);

        run_detached(&ctx, &mut Vec::new());

        // The child runs concurrently; poll briefly for its side effect.
        let mut seen = false;
        for _ in 0..50 {
            if cwd.join("post.txt").exists() {
                seen = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(seen, "detached command never produced its file");
    }

    #[test]
    fn does_nothing_without_a_configured_hook() {
        let (_dir, cwd) = scratch();
        let cfg = config("");
        let ctx = ctx(HookPhase::PreRemove, &cfg, "ws", &cwd);

        run_blocking(&ctx).unwrap();
        run_detached(&ctx, &mut Vec::new());

        assert_eq!(std::fs::read_dir(cwd).unwrap().count(), 0);
    }
}
