//! `tt switch [name]` — resolve the destination workspace and record the
//! previous one. The shell wrapper performs the actual `cd`.
//!
//! Output contract: the destination path goes to real stdout (the wrapper
//! captures it); the picker menu goes to stderr so it never pollutes `cd`.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::cmd::create::{self, CreateError};
use crate::config::schema::UserConfig;
use crate::hook::{self, HookContext, HookError, HookPhase};
use crate::jj::{Jj, JjError, WorkspaceInfo};
use crate::state::{StateError, WorkspaceState};

/// `-` is reserved for "previous workspace".
const PREVIOUS_NAME: &str = "-";

/// Everything `tt switch` needs from its caller: the inputs vary per
/// invocation, the jj adapter and state store are injected separately.
#[derive(Debug, Clone)]
pub struct SwitchArgs<'a> {
    pub name: Option<&'a str>,
    /// `--create <name>`: create the workspace, then switch to it.
    pub create: bool,
    /// The loaded user config: `workspace-path` template and lifecycle
    /// hooks. `None` = defaults, no hooks.
    pub config: Option<&'a UserConfig>,
    pub current_root: &'a Path,
    pub home: &'a Path,
}

pub fn run(
    jj: &dyn Jj,
    args: SwitchArgs<'_>,
    state: &dyn WorkspaceState,
    menu_out: &mut dyn Write,
    picker_in: &mut dyn BufRead,
) -> Result<PathBuf, SwitchError> {
    let workspaces = jj.workspaces()?;
    let current = workspaces
        .iter()
        .find(|w| w.is_current)
        .ok_or(SwitchError::CurrentWorkspaceNotFound)?;

    let target: WorkspaceInfo = match (args.create, args.name) {
        (true, Some(name)) => {
            create::run(
                jj,
                name,
                args.config.and_then(|c| c.workspace_path()),
                args.current_root,
                args.home,
                args.config,
            )?;
            // Re-list: the fresh workspace must exist before we switch to it.
            let fresh = jj.workspaces()?;
            resolve(&fresh, name)?.clone()
        }
        (true, None) => return Err(SwitchError::CreateNeedsName),
        (false, None) => pick(&workspaces, menu_out, picker_in)?.clone(),
        (false, Some(PREVIOUS_NAME)) => {
            let previous = state
                .previous_workspace()?
                .ok_or(SwitchError::NoPreviousWorkspace)?;
            resolve(&workspaces, &previous)?.clone()
        }
        (false, Some(name)) => resolve(&workspaces, name)?.clone(),
    };

    if target.name == current.name {
        // Switching in place: the previous workspace must stay the one we
        // could still come back from, and nothing moves — only post-switch
        // fires.
        if let Some(config) = args.config {
            hook::run_detached(
                &HookContext {
                    phase: HookPhase::PostSwitch,
                    workspace: &current.name,
                    workspace_path: current.root.as_deref().unwrap_or(args.current_root),
                    config,
                },
                menu_out,
            );
        }
        return destination(current);
    }

    // pre-switch guards the actual move (blocking); worktrunk parity: runs
    // in the source workspace.
    if let Some(config) = args.config {
        hook::run_blocking(&HookContext {
            phase: HookPhase::PreSwitch,
            workspace: &current.name,
            workspace_path: args.current_root,
            config,
        })
        .map_err(SwitchError::Hook)?;
    }

    state.set_previous_workspace(Some(&current.name))?;
    let destination = destination(&target)?;

    if let Some(config) = args.config {
        hook::run_detached(
            &HookContext {
                phase: HookPhase::PostSwitch,
                workspace: &target.name,
                workspace_path: target.root.as_deref().unwrap_or(args.current_root),
                config,
            },
            menu_out,
        );
    }
    Ok(destination)
}

fn resolve<'w>(
    workspaces: &'w [WorkspaceInfo],
    name: &str,
) -> Result<&'w WorkspaceInfo, SwitchError> {
    workspaces.iter().find(|w| w.name == name).ok_or_else(|| {
        let mut available = workspaces
            .iter()
            .map(|w| w.name.as_str())
            .collect::<Vec<_>>();
        available.sort();
        SwitchError::UnknownWorkspace {
            name: name.to_string(),
            available: available.join(", "),
        }
    })
}

fn destination(workspace: &WorkspaceInfo) -> Result<PathBuf, SwitchError> {
    workspace
        .root
        .clone()
        .ok_or_else(|| SwitchError::RootNotRecorded {
            name: workspace.name.clone(),
        })
}

/// One-shot numbered picker on `menu_out`; the choice is read from
/// `picker_in`. A single attempt — EOF or invalid input cancels.
fn pick<'w>(
    workspaces: &'w [WorkspaceInfo],
    menu_out: &mut dyn Write,
    picker_in: &mut dyn BufRead,
) -> Result<&'w WorkspaceInfo, SwitchError> {
    for (index, w) in workspaces.iter().enumerate() {
        let marker = if w.is_current { "*" } else { " " };
        let root = w
            .root
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".to_string());
        writeln!(menu_out, "{}) {} {}  {}", index + 1, marker, w.name, root)
            .map_err(|source| SwitchError::MenuWrite { source })?;
    }
    write!(menu_out, "switch to [1-{}]: ", workspaces.len())
        .map_err(|source| SwitchError::MenuWrite { source })?;
    menu_out
        .flush()
        .map_err(|source| SwitchError::MenuWrite { source })?;

    let mut line = String::new();
    let read = picker_in
        .read_line(&mut line)
        .map_err(|source| SwitchError::ReadInput { source })?;
    if read == 0 {
        return Err(SwitchError::PickerCancelled);
    }
    let input = line.trim().to_string();
    match input.parse::<usize>() {
        Ok(choice) if choice >= 1 && choice <= workspaces.len() => Ok(&workspaces[choice - 1]),
        _ => Err(SwitchError::InvalidChoice {
            input,
            max: workspaces.len(),
        }),
    }
}

#[derive(Debug, Error)]
pub enum SwitchError {
    #[error(transparent)]
    Jj(#[from] JjError),
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Create(#[from] CreateError),
    #[error("`--create` needs a workspace name")]
    CreateNeedsName,
    #[error(transparent)]
    Hook(#[from] HookError),
    #[error("no workspace named `{name}` (available: {available})")]
    UnknownWorkspace { name: String, available: String },
    #[error("no previous workspace recorded — use `tt switch <name>` first")]
    NoPreviousWorkspace,
    #[error("cannot determine the current workspace")]
    CurrentWorkspaceNotFound,
    #[error(
        "workspace `{name}` has no recorded root (created with jj < 0.38) — cannot switch to it"
    )]
    RootNotRecorded { name: String },
    #[error("cancelled — nothing chosen")]
    PickerCancelled,
    #[error("invalid choice `{input}`; expected 1..={max}")]
    InvalidChoice { input: String, max: usize },
    #[error("cannot write the picker menu: {source}")]
    MenuWrite {
        #[source]
        source: std::io::Error,
    },
    #[error("cannot read the picker choice: {source}")]
    ReadInput {
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod switch_should {
    use super::*;
    use crate::config::schema::UserConfig;
    use crate::jj::WcStatus;
    use pretty_assertions::assert_eq;
    use std::cell::RefCell;
    use std::io::Cursor;

    struct FakeJj {
        workspaces: Vec<WorkspaceInfo>,
    }

    impl Jj for FakeJj {
        fn workspace_root(&self) -> Result<PathBuf, JjError> {
            panic!("FakeJj::workspace_root is not exercised by switch tests")
        }

        fn workspaces(&self) -> Result<Vec<WorkspaceInfo>, JjError> {
            Ok(self.workspaces.clone())
        }

        fn workspace_add(
            &self,
            _name: &str,
            _destination: &std::path::Path,
        ) -> Result<(), JjError> {
            panic!("FakeJj::workspace_add is not exercised by switch tests")
        }

        fn bookmark_create(&self, _name: &str, _workspace: &str) -> Result<(), JjError> {
            panic!("FakeJj::bookmark_create is not exercised by switch tests")
        }

        fn workspace_forget(&self, _name: &str) -> Result<(), JjError> {
            panic!("FakeJj::workspace_forget is not exercised by switch tests")
        }

        fn bookmark_delete(&self, _name: &str) -> Result<bool, JjError> {
            panic!("FakeJj::bookmark_delete is not exercised by switch tests")
        }
    }

    struct FakeState {
        previous: RefCell<Option<String>>,
        set_calls: RefCell<Vec<Option<String>>>,
    }

    impl FakeState {
        fn with_previous(name: &str) -> Self {
            Self {
                previous: RefCell::new(Some(name.to_string())),
                set_calls: RefCell::new(Vec::new()),
            }
        }

        fn empty() -> Self {
            Self {
                previous: RefCell::new(None),
                set_calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl WorkspaceState for FakeState {
        fn previous_workspace(&self) -> Result<Option<String>, StateError> {
            Ok(self.previous.borrow().clone())
        }

        fn set_previous_workspace(&self, name: Option<&str>) -> Result<(), StateError> {
            *self.previous.borrow_mut() = name.map(str::to_string);
            self.set_calls.borrow_mut().push(name.map(str::to_string));
            Ok(())
        }
    }

    fn ws(name: &str, current: bool, root: &str) -> WorkspaceInfo {
        WorkspaceInfo {
            name: name.to_string(),
            root: Some(PathBuf::from(root)),
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

    fn repo() -> FakeJj {
        FakeJj {
            workspaces: vec![
                ws("default", true, "/repo/default"),
                ws("ws2", false, "/repo/ws2"),
            ],
        }
    }

    fn switch_from(
        jj: &dyn Jj,
        name: Option<&str>,
        state: &dyn WorkspaceState,
        input: &str,
    ) -> (Result<PathBuf, SwitchError>, Vec<u8>) {
        switch_with(
            jj,
            SwitchArgs {
                name,
                create: false,
                config: None,
                current_root: Path::new("/repo/default"),
                home: Path::new("/home/u"),
            },
            state,
            input,
        )
    }

    fn switch_with(
        jj: &dyn Jj,
        args: SwitchArgs<'_>,
        state: &dyn WorkspaceState,
        input: &str,
    ) -> (Result<PathBuf, SwitchError>, Vec<u8>) {
        let mut menu = Vec::new();
        let mut picker_in = Cursor::new(input.to_string());
        let result = run(jj, args, state, &mut menu, &mut picker_in);
        (result, menu)
    }

    #[test]
    fn resolves_a_workspace_by_name() {
        let (result, _) = switch_from(&repo(), Some("ws2"), &FakeState::empty(), "");

        assert_eq!(result.unwrap(), PathBuf::from("/repo/ws2"));
    }

    #[test]
    fn fails_when_creating_without_a_name() {
        let (result, _) = switch_with(
            &repo(),
            SwitchArgs {
                name: None,
                create: true,
                config: None,
                current_root: Path::new("/repo/default"),
                home: Path::new("/home/u"),
            },
            &FakeState::empty(),
            "",
        );

        assert!(matches!(result.unwrap_err(), SwitchError::CreateNeedsName));
    }

    #[test]
    fn a_failing_pre_switch_aborts_before_touching_the_state() {
        let scratch = tempfile::tempdir().unwrap();
        let config: UserConfig = toml::from_str("pre-switch = \"exit 1\"").unwrap();
        let state = FakeState::with_previous("ws2");

        let (result, _) = switch_with(
            &repo(),
            SwitchArgs {
                name: Some("ws2"),
                create: false,
                config: Some(&config),
                current_root: scratch.path(),
                home: Path::new("/home/u"),
            },
            &state,
            "",
        );

        let error = result.unwrap_err();
        assert!(
            matches!(error, SwitchError::Hook(_)),
            "expected the hook failure, got: {error:?}"
        );
        assert!(
            state.set_calls.borrow().is_empty(),
            "the previous workspace must not be recorded when pre-switch fails"
        );
    }

    #[test]
    fn records_the_workspace_we_left() {
        let state = FakeState::empty();

        let (result, _) = switch_from(&repo(), Some("ws2"), &state, "");
        result.unwrap();

        assert_eq!(*state.set_calls.borrow(), vec![Some("default".to_string())]);
    }

    #[test]
    fn does_not_touch_the_previous_state_when_switching_in_place() {
        let state = FakeState::with_previous("ws2");

        let (result, _) = switch_from(&repo(), Some("default"), &state, "");
        result.unwrap();

        assert!(state.set_calls.borrow().is_empty());
        assert_eq!(
            state.previous_workspace().unwrap(),
            Some("ws2".to_string()),
            "in-place switch must keep the escape route to `ws2`"
        );
    }

    #[test]
    fn goes_back_to_the_previous_workspace_on_dash() {
        let state = FakeState::with_previous("ws2");

        let (result, _) = switch_from(&repo(), Some("-"), &state, "");

        assert_eq!(result.unwrap(), PathBuf::from("/repo/ws2"));
        assert_eq!(*state.set_calls.borrow(), vec![Some("default".to_string())]);
    }

    #[test]
    fn fails_for_an_unknown_workspace_listing_the_available_ones() {
        let (result, _) = switch_from(&repo(), Some("nope"), &FakeState::empty(), "");

        match result.unwrap_err() {
            SwitchError::UnknownWorkspace { name, available } => {
                assert_eq!(name, "nope");
                assert_eq!(available, "default, ws2");
            }
            other => panic!("expected UnknownWorkspace, got: {other:?}"),
        }
    }

    #[test]
    fn fails_when_no_previous_workspace_was_recorded() {
        let (result, _) = switch_from(&repo(), Some("-"), &FakeState::empty(), "");

        assert!(matches!(
            result.unwrap_err(),
            SwitchError::NoPreviousWorkspace
        ));
    }

    #[test]
    fn picks_the_chosen_entry_from_the_menu() {
        let (result, menu) = switch_from(&repo(), None, &FakeState::empty(), "2\n");

        assert_eq!(result.unwrap(), PathBuf::from("/repo/ws2"));
        let menu = String::from_utf8(menu).unwrap();
        assert!(menu.contains("1) * default"), "menu was: {menu:?}");
        assert!(menu.contains("2)   ws2"), "menu was: {menu:?}");
    }

    #[test]
    fn cancels_when_the_menu_input_ends_immediately() {
        let (result, _) = switch_from(&repo(), None, &FakeState::empty(), "");

        assert!(matches!(result.unwrap_err(), SwitchError::PickerCancelled));
    }

    #[test]
    fn rejects_a_choice_out_of_range() {
        let (result, _) = switch_from(&repo(), None, &FakeState::empty(), "5\n");

        match result.unwrap_err() {
            SwitchError::InvalidChoice { input, max } => {
                assert_eq!(input, "5");
                assert_eq!(max, 2);
            }
            other => panic!("expected InvalidChoice, got: {other:?}"),
        }
    }
}
