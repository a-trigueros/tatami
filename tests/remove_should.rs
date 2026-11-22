//! Proof tier: `tt remove` against a real jj repo.

mod common;

use common::TempRepo;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tatami::cmd::remove::{self, DirectoryOutcome, RemoveArgs};
use tatami::jj::{CliJj, Jj};

fn remove_args<'a>(name: &'a str, delete_bookmark: bool, state_dir: &'a Path) -> RemoveArgs<'a> {
    RemoveArgs {
        name,
        delete_bookmark,
        now_millis: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64,
        state_dir,
        config: None,
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
    let report = remove::run(
        &jj,
        remove_args("ws2", false, &state_dir.path().join(".jj/tatami")),
        &mut Vec::new(),
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

    let error = remove::run(
        &jj,
        remove_args("default", false, &state_dir.path().join(".jj/tatami")),
        &mut Vec::new(),
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
    let report = remove::run(
        &jj,
        remove_args("ws2", true, &state_dir.path().join(".jj/tatami")),
        &mut Vec::new(),
    )
    .expect("remove ws2 with bookmark");

    assert!(report.bookmark_deleted);
    let bookmarks = repo.capture_jj(&["bookmark", "list"]);
    assert!(
        !bookmarks.contains("ws2"),
        "bookmark should be gone, list was: {bookmarks:?}"
    );
}
