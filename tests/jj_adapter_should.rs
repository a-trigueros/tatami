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
