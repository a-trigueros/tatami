//! Hook execution over the user and project config lifecycle hooks (PLAN §4
//! shapes).
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
//! Hooks accumulate: the user's hooks run first, then the project's
//! (worktrunk-sourced order). Project hooks are gated by an
//! [`ApprovalGate`] — every command is approved (or denied) **before any
//! project command runs**, keyed on the raw template so a changed command
//! re-requests approval.
//!
//! Template variables today: `{{ workspace }}`, `{{ workspace_path }}`.
//! shortcut: the full minijinja variable/filter set lands in M4.

use std::io::Write;
use std::path::Path;
use std::process::Command;

use thiserror::Error;

use crate::config::schema::{CommandSpec, ProjectConfig, UserConfig};

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

/// Decides whether a project-hook command may run. The real gate prompts;
/// fakes answer from a script. Takes `&mut` because approving persists the
/// record.
pub trait ApprovalGate {
    /// `true` when the command may run.
    fn approve(&mut self, phase: HookPhase, command: &str) -> bool;
}

/// Approves everything — tests and flows that carry no project config.
pub struct NoGate;

impl ApprovalGate for NoGate {
    fn approve(&mut self, _phase: HookPhase, _command: &str) -> bool {
        true
    }
}

/// The resolved hook sources for one operation. User hooks run first, then
/// project hooks (worktrunk-sourced accumulation). The gate travels
/// separately so it can own its IO channels.
#[derive(Debug)]
pub struct Hooks<'a> {
    pub user: Option<&'a UserConfig>,
    pub project: Option<&'a ProjectConfig>,
}

impl Hooks<'_> {
    /// No hooks at all.
    pub fn none() -> Hooks<'static> {
        Hooks {
            user: None,
            project: None,
        }
    }

    fn specs(&self, phase: HookPhase) -> Vec<(Origin, &CommandSpec)> {
        let mut specs = Vec::new();
        if let Some(user) = self.user
            && let Some(spec) = user.hook(phase)
        {
            specs.push((Origin::User, spec));
        }
        if let Some(project) = self.project
            && let Some(spec) = project.hook(phase)
        {
            specs.push((Origin::Project, spec));
        }
        specs
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    User,
    Project,
}

/// Everything a hook template can render and run against.
pub struct HookContext<'a> {
    pub phase: HookPhase,
    pub workspace: &'a str,
    pub workspace_path: &'a Path,
    pub hooks: &'a Hooks<'a>,
}

impl HookContext<'_> {
    fn render(&self, command: &str) -> String {
        command.replace("{{ workspace }}", self.workspace).replace(
            "{{ workspace_path }}",
            &self.workspace_path.display().to_string(),
        )
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
    /// A project hook was denied — the operation must abort without running
    /// the denied command.
    #[error("project `{phase}` hook `{command}` was not approved")]
    Denied {
        phase: &'static str,
        command: String,
    },
}

/// Runs a `pre-*` phase: user hooks first, then project hooks (each project
/// command gated by approval before any project command runs). The first
/// failing command aborts.
pub fn run_blocking(ctx: &HookContext<'_>, gate: &mut dyn ApprovalGate) -> Result<(), HookError> {
    for (origin, spec) in ctx.hooks.specs(ctx.phase) {
        if origin == Origin::Project {
            for (_name, command) in entries(spec) {
                if !gate.approve(ctx.phase, &command) {
                    return Err(HookError::Denied {
                        phase: ctx.phase.config_key(),
                        command,
                    });
                }
            }
        }
        execute_blocking(spec, ctx, ctx.workspace_path)?;
    }
    Ok(())
}

/// Runs a `post-*` phase: commands spawn detached, failures are reported on
/// `out` without failing the caller's operation.
pub fn run_detached(ctx: &HookContext<'_>, out: &mut dyn Write, gate: &mut dyn ApprovalGate) {
    for (origin, spec) in ctx.hooks.specs(ctx.phase) {
        if origin == Origin::Project {
            let mut denied = false;
            for (_name, command) in entries(spec) {
                if !gate.approve(ctx.phase, &command) {
                    let _ = writeln!(
                        out,
                        "tatami: warning: project `{}` hook `{command}` was not approved — skipped",
                        ctx.phase.config_key()
                    );
                    denied = true;
                }
            }
            if denied {
                continue;
            }
        }
        if let Err(error) = execute_detached(spec, ctx, ctx.workspace_path) {
            let _ = writeln!(out, "tatami: warning: {}", error);
        }
    }
}

/// [`run_detached`] for flows without a warning channel (standalone
/// creation): warnings are discarded.
pub fn run_detached_quiet(ctx: &HookContext<'_>, gate: &mut dyn ApprovalGate) {
    let mut sink = std::io::sink();
    run_detached(ctx, &mut sink, gate);
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
    use std::cell::RefCell;
    use std::path::PathBuf;

    fn user_config(raw: &str) -> UserConfig {
        toml::from_str(raw).unwrap()
    }

    fn project_config(raw: &str) -> ProjectConfig {
        toml::from_str(raw).unwrap()
    }

    /// A gate that answers from a script and records what it was asked.
    struct ScriptedGate {
        answers: RefCell<Vec<bool>>,
        asked: RefCell<Vec<String>>,
    }

    impl ScriptedGate {
        fn denying_first(then_approve: bool) -> Self {
            Self {
                answers: RefCell::new(vec![false, then_approve]),
                asked: RefCell::new(Vec::new()),
            }
        }
    }

    impl ApprovalGate for ScriptedGate {
        fn approve(&mut self, _phase: HookPhase, command: &str) -> bool {
            self.asked.borrow_mut().push(command.to_string());
            self.answers.borrow_mut().pop().unwrap_or(true)
        }
    }

    fn ctx<'a>(
        phase: HookPhase,
        hooks: &'a Hooks<'a>,
        workspace: &'a str,
        workspace_path: &'a Path,
    ) -> HookContext<'a> {
        HookContext {
            phase,
            workspace,
            workspace_path,
            hooks,
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
        let user = user_config("pre-start = \"echo started > marker.txt\"");
        let hooks = Hooks {
            user: Some(&user),
            project: None,
        };
        let ctx = ctx(HookPhase::PreStart, &hooks, "ws2", &cwd);
        let mut gate = NoGate;

        run_blocking(&ctx, &mut gate).unwrap();

        assert_eq!(
            std::fs::read_to_string(cwd.join("marker.txt")).unwrap(),
            "started\n"
        );
    }

    #[test]
    fn expands_the_workspace_variables() {
        let (_dir, cwd) = scratch();
        let user =
            user_config("pre-start = \"echo {{ workspace }} at {{ workspace_path }} > out.txt\"");
        let hooks = Hooks {
            user: Some(&user),
            project: None,
        };
        let ctx = ctx(HookPhase::PreStart, &hooks, "feature", &cwd);
        let mut gate = NoGate;

        run_blocking(&ctx, &mut gate).unwrap();

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
        let user = user_config(
            r#"
            [pre-start]
            one = "echo 1 > one.txt"
            two = "echo 2 > two.txt"
            "#,
        );
        let hooks = Hooks {
            user: Some(&user),
            project: None,
        };
        let ctx = ctx(HookPhase::PreStart, &hooks, "ws", &cwd);
        let mut gate = NoGate;

        run_blocking(&ctx, &mut gate).unwrap();

        assert!(cwd.join("one.txt").exists());
        assert!(cwd.join("two.txt").exists());
    }

    #[test]
    fn stops_a_pipeline_at_the_first_failing_entry() {
        let (_dir, cwd) = scratch();
        let user = user_config(
            r#"
            [[pre-start]]
            broken = "exit 3"

            [[pre-start]]
            never = "echo unreachable > never.txt"
            "#,
        );
        let hooks = Hooks {
            user: Some(&user),
            project: None,
        };
        let ctx = ctx(HookPhase::PreStart, &hooks, "ws", &cwd);
        let mut gate = NoGate;

        let error = run_blocking(&ctx, &mut gate).unwrap_err();

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
    fn runs_user_hooks_before_project_hooks() {
        let (_dir, cwd) = scratch();
        let user = user_config("pre-start = \"echo user > order.txt\"");
        let project = project_config("pre-start = \"echo project >> order.txt\"");
        let hooks = Hooks {
            user: Some(&user),
            project: Some(&project),
        };
        let ctx = ctx(HookPhase::PreStart, &hooks, "ws", &cwd);
        let mut gate = NoGate;

        run_blocking(&ctx, &mut gate).unwrap();

        let order = std::fs::read_to_string(cwd.join("order.txt")).unwrap();
        assert_eq!(order, "user\nproject\n");
    }

    #[test]
    fn gates_every_project_command_before_running_any_of_them() {
        let (_dir, cwd) = scratch();
        let project = project_config(
            r#"
            [[pre-start]]
            first = "echo first > first.txt"

            [[pre-start]]
            second = "echo second > second.txt"
            "#,
        );
        let hooks = Hooks {
            user: None,
            project: Some(&project),
        };
        let mut gate = ScriptedGate::denying_first(true);
        let ctx = ctx(HookPhase::PreStart, &hooks, "ws", &cwd);

        let error = run_blocking(&ctx, &mut gate).unwrap_err();

        match error {
            HookError::Denied { phase, command } => {
                assert_eq!(phase, "pre-start");
                assert_eq!(command, "echo second > second.txt");
            }
            other => panic!("expected Denied, got: {other:?}"),
        }
        assert!(
            !cwd.join("first.txt").exists(),
            "no project command may run when one is denied"
        );
    }

    #[test]
    fn detaches_post_commands_without_blocking() {
        let (_dir, cwd) = scratch();
        let user = user_config("post-start = \"echo detached > post.txt\"");
        let hooks = Hooks {
            user: Some(&user),
            project: None,
        };
        let ctx = ctx(HookPhase::PostStart, &hooks, "ws", &cwd);
        let mut gate = NoGate;

        run_detached(&ctx, &mut Vec::new(), &mut gate);

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
        let hooks = Hooks::none();
        let ctx = ctx(HookPhase::PreRemove, &hooks, "ws", &cwd);
        let mut gate = NoGate;

        run_blocking(&ctx, &mut gate).unwrap();
        run_detached(&ctx, &mut Vec::new(), &mut gate);

        assert_eq!(std::fs::read_dir(cwd).unwrap().count(), 0);
    }
}
