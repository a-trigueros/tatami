//! Proof tier: `tt remove` against a real jj repo.

mod common;

use common::TempRepo;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use tatami::cmd::remove::{self, DirectoryOutcome, RemoveArgs};
use tatami::hook::{Hooks, NoGate};
use tatami::jj::{CliJj, Jj};

fn remove_args<'a>(
    name: &'a str,
    delete_bookmark: bool,
    state_dir: &'a Path,
    hooks: &'a Hooks<'a>,
) -> RemoveArgs<'a> {
    RemoveArgs {
        name,
        delete_bookmark,
        now_millis: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64,
        state_dir,
        hooks,
        caller_root: state_dir,
    }
}

#[test]
fn forgets_the_workspace_and_trashes_its_directory() {
    let repo = TempRepo::init();
    repo.add_workspace("ws2");
    let ws2_root = repo.path().parent().unwrap().join("ws2");
    // Ignored-style file: lives only in the workspace directory, hence trash.
    fs::create_dir_all(ws2_root.join(".env")).unwrap();
    fs::write(ws2_root.join(".env/local"), "SECRET=1").unwrap();

    let jj = CliJj::at(repo.path().clone());
    let state_dir = tempfile::tempdir().unwrap();
    let hooks = Hooks::none();
    let report = remove::run(
        &jj,
        remove_args("ws2", false, &state_dir.path().join(".jj/tatami"), &hooks),
        &mut Vec::new(),
        &mut NoGate,
    )
    .expect("remove ws2");

    // jj no longer tracks it.
    let names: Vec<_> = jj
        .workspaces()
        .unwrap()
        .into_iter()
        .map(|w| w.name)
        .collect();
    assert!(!names.contains(&"ws2".to_string()), "workspaces: {names:?}");

    let trashed = match report.directory {
        DirectoryOutcome::Trashed(path) => path,
        other => panic!("expected Trashed, got: {other:?}"),
    };
    assert!(
        trashed.join(".env/local").exists(),
        "workspace contents must survive in the trash"
    );
    assert!(!ws2_root.exists());
}

#[test]
fn refuses_the_current_workspace_and_the_host() {
    let repo = TempRepo::init();

    let jj = CliJj::at(repo.path().clone());
    let state_dir = tempfile::tempdir().unwrap();
    let hooks = Hooks::none();

    let error = remove::run(
        &jj,
        remove_args(
            "default",
            false,
            &state_dir.path().join(".jj/tatami"),
            &hooks,
        ),
        &mut Vec::new(),
        &mut NoGate,
    )
    .unwrap_err();

    assert!(
        matches!(error, remove::RemoveError::RefuseCurrentWorkspace { .. }),
        "the current workspace is also the host here, but standing in it wins: {error:?}"
    );
}

#[test]
fn deletes_an_existing_namesake_bookmark_when_asked() {
    let repo = TempRepo::init();
    repo.add_workspace("ws2");
    repo.capture_jj(&["bookmark", "create", "ws2", "-r", "ws2@"]);

    let jj = CliJj::at(repo.path().clone());
    let state_dir = tempfile::tempdir().unwrap();
    let hooks = Hooks::none();
    remove::run(
        &jj,
        remove_args("ws2", true, &state_dir.path().join(".jj/tatami"), &hooks),
        &mut Vec::new(),
        &mut NoGate,
    )
    .expect("remove ws2 with bookmark");

    // The bookmark may be deleted by tatami's explicit call OR by jj's
    // abandon (abandoning a commit deletes bookmarks pointing exactly at
    // it); either way the outcome is: gone.
    let bookmarks = repo.capture_jj(&["bookmark", "list"]);
    assert!(
        !bookmarks.contains("ws2"),
        "bookmark should be gone, list was: {bookmarks:?}"
    );
}

#[test]
fn removes_a_workspace_whose_name_contains_a_slash() {
    // Real failure: `remove feat/test` trashed to `trash/feat/test-…` and
    // rename(2) failed with ENOENT (`trash/feat/` does not exist).
    let repo = TempRepo::init();
    repo.describe_working_copy("main workspace");
    // Create the slashed workspace the way `switch --create` does, with
    // config isolation: tests must never read the machine's real tatami or
    // worktrunk config (its template points at ~/factory).
    let home = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tatami"))
        .args(["switch", "--create", "feat/test"])
        .current_dir(repo.path())
        .env("JJ_CONFIG", repo.jj_config_path())
        .env("HOME", home.path())
        .output()
        .expect("switch --create feat/test");
    assert!(
        output.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Default layout: a sibling dir named `<repo>.feat-test`.
    let repo_name = repo
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let ws_root = repo
        .path()
        .parent()
        .unwrap()
        .join(format!("{repo_name}.feat-test"));
    assert!(ws_root.exists(), "default layout dir must exist");

    // forget(0.47.1) does NOT abandon the removed workspace's working-copy
    // commit: without the explicit abandon, an empty change lingers forever.
    let ws_at = repo.capture_jj(&[
        "log",
        "--no-graph",
        "-T",
        "change_id.short()",
        "-r",
        "feat/test@",
    ]);
    assert!(!ws_at.is_empty(), "the ws @ must exist before remove");

    let jj = CliJj::at(repo.path().clone());
    let state_dir = tempfile::tempdir().unwrap();
    let hooks = Hooks::none();
    let report = remove::run(
        &jj,
        remove_args("feat/test", true, state_dir.path(), &hooks),
        &mut Vec::new(),
        &mut NoGate,
    )
    .expect("remove feat/test");

    match report.directory {
        DirectoryOutcome::Trashed(destination) => {
            let entry = destination.file_name().and_then(|n| n.to_str()).unwrap();
            assert!(
                entry.starts_with("feat-test-"),
                "sanitized trash entry (no `trash/feat/` subdirectory needed): {entry:?}"
            );
            assert!(destination.is_dir());
        }
        other => panic!("expected Trashed, got: {other:?}"),
    }
    assert!(!ws_root.exists(), "the workspace dir must be gone");
    // Default since the flip: the namesake bookmark goes with the workspace.
    let bookmarks = repo.capture_jj(&["bookmark", "list"]);
    assert!(
        !bookmarks.contains("feat/test"),
        "the namesake bookmark must be deleted by default: {bookmarks:?}"
    );
    let log = repo.capture_jj(&["log", "--no-graph", "-T", "change_id.short()"]);
    assert!(
        !log.contains(ws_at.trim()),
        "the removed workspace's @ must be abandoned: {log:?} still has {ws_at:?}"
    );
}

#[test]
fn an_unapproved_project_pre_remove_hook_aborts_the_removal() {
    let repo = TempRepo::init();
    repo.add_workspace("ws2");

    let project: tatami::config::ProjectConfig =
        toml::from_str("pre-remove = \"rm -rf .\"\n").unwrap();
    let hooks = Hooks {
        user: None,
        project: Some(&project),
    };
    // A gate that denies everything: no approval given.
    struct Deny;
    impl tatami::hook::ApprovalGate for Deny {
        fn approve(&mut self, _key: &str, _command: &str) -> bool {
            false
        }
    }

    let jj = CliJj::at(repo.path().clone());
    let state_dir = tempfile::tempdir().unwrap();
    let error = remove::run(
        &jj,
        remove_args("ws2", false, &state_dir.path().join(".jj/tatami"), &hooks),
        &mut Vec::new(),
        &mut Deny,
    )
    .unwrap_err();

    assert!(
        matches!(error, remove::RemoveError::Hook(_)),
        "denial must abort: {error:?}"
    );
    // Nothing happened: workspace still tracked, directory still there.
    let names: Vec<_> = jj
        .workspaces()
        .unwrap()
        .into_iter()
        .map(|w| w.name)
        .collect();
    assert!(names.contains(&"ws2".to_string()));
    assert!(repo.path().parent().unwrap().join("ws2").exists());
}
