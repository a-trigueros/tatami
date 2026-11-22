//! Proof tier: `tt switch` against a real jj repo.

mod common;

use common::TempRepo;
use std::io::Cursor;
use std::path::Path;
use tatami::cmd::switch::{self, SwitchArgs};
use tatami::hook::{Hooks, NoGate};
use tatami::jj::CliJj;
use tatami::state::{FileState, WorkspaceState};

static NO_HOOKS: Hooks<'static> = Hooks {
    user: None,
    project: None,
};

fn args<'a>(root: &'a Path, name: Option<&'a str>, create: bool) -> SwitchArgs<'a> {
    SwitchArgs {
        name,
        create,
        hooks: &NO_HOOKS,
        current_root: root,
        home: Path::new("/home/u"),
        logs_dir: None,
    }
}

#[test]
fn switches_between_workspaces_and_records_the_previous_one() {
    let repo = TempRepo::init();
    repo.describe_working_copy("main workspace");
    repo.add_workspace("ws2");

    let jj = CliJj::at(repo.path().clone());
    let state = FileState::for_workspace(&repo.path()).unwrap();
    let mut menu = Vec::new();

    let dest = switch::run(
        &jj,
        args(&repo.path(), Some("ws2"), false),
        &state,
        &mut menu,
        &mut Cursor::new(String::new()),
        &mut NoGate,
    )
    .expect("switch to ws2");
    let ws2_root = repo.path().parent().unwrap().join("ws2");
    assert_eq!(dest, ws2_root);
    assert_eq!(
        state.previous_workspace().unwrap(),
        Some("default".to_string())
    );

    // The shell has now moved to ws2; tatami runs from there, and the state —
    // repo-global — is found again through the pointer file.
    let jj_from_ws2 = CliJj::at(ws2_root.clone());
    let state_from_ws2 = FileState::for_workspace(&ws2_root).unwrap();
    let dest = switch::run(
        &jj_from_ws2,
        args(&ws2_root, Some("-"), false),
        &state_from_ws2,
        &mut menu,
        &mut Cursor::new(String::new()),
        &mut NoGate,
    )
    .expect("switch back with -");
    assert_eq!(dest, repo.path());
    assert_eq!(
        state_from_ws2.previous_workspace().unwrap(),
        Some("ws2".to_string())
    );
}

#[test]
fn switches_in_place_without_touching_the_previous_state() {
    let repo = TempRepo::init();
    repo.add_workspace("ws2");

    let jj = CliJj::at(repo.path().clone());
    let state = FileState::for_workspace(&repo.path()).unwrap();
    let mut menu = Vec::new();

    // Establish `ws2` as the previous workspace first.
    switch::run(
        &jj,
        args(&repo.path(), Some("ws2"), false),
        &state,
        &mut menu,
        &mut Cursor::new(String::new()),
        &mut NoGate,
    )
    .unwrap();

    let dest = switch::run(
        &jj,
        args(&repo.path(), Some("default"), false),
        &state,
        &mut menu,
        &mut Cursor::new(String::new()),
        &mut NoGate,
    )
    .expect("switch to current workspace");

    assert_eq!(dest, repo.path());
    assert_eq!(
        state.previous_workspace().unwrap(),
        Some("default".to_string()),
        "switching in place must not overwrite the escape route"
    );
}
