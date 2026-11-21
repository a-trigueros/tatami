mod common;

use common::TempRepo;
use tatami::jj::{CliJj, Jj, JjError};

#[test]
fn reports_the_workspace_root_of_a_fresh_repo() {
    let repo = TempRepo::init();

    let jj = CliJj::at(repo.path().clone());
    let root = jj.workspace_root().unwrap();

    assert_eq!(root, repo.path());
}

#[test]
fn carries_jj_diagnosis_when_running_outside_a_repo() {
    let repo = TempRepo::init();

    let jj = CliJj::at(repo.outside_dir().to_path_buf());
    let error = jj.workspace_root().unwrap_err();

    match error {
        JjError::CommandFailed {
            command,
            stderr,
            status,
        } => {
            assert!(command.contains("workspace root"), "command was: {command}");
            assert!(stderr.contains("no jj repo"), "stderr was: {stderr}");
            assert!(!status.success());
        }
        other => panic!("expected CommandFailed, got: {other:?}"),
    }
}

#[test]
fn lists_workspaces_with_root_and_working_copy_status() {
    let repo = TempRepo::init();
    repo.describe_working_copy("hello main");
    repo.add_workspace("ws2");

    let jj = CliJj::at(repo.path().clone());
    let workspaces = jj.workspaces().unwrap();

    let default = workspaces
        .iter()
        .find(|w| w.name == "default")
        .expect("default workspace listed");
    assert!(default.is_current);
    assert_eq!(default.root, Some(repo.path()));
    assert!(default.wc.empty);
    assert_eq!(default.wc.description, "hello main");
    assert!(!default.wc.change.is_empty());

    let ws2 = workspaces
        .iter()
        .find(|w| w.name == "ws2")
        .expect("secondary workspace listed");
    assert!(!ws2.is_current);
    assert_eq!(ws2.root, Some(repo.path().parent().unwrap().join("ws2")));
    assert!(ws2.wc.empty);
    assert_eq!(ws2.wc.description, "");
}
