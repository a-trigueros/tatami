//! Proof tier: `tt step copy-ignored` against a real jj repo.

mod common;

use common::TempRepo;
use tatami::cmd::copy_ignored::{self, CopyIgnoredArgs};
use tatami::jj::CliJj;

#[test]
fn copies_ignored_but_not_untracked_or_tracked_files() {
    let repo = TempRepo::init();
    repo.add_workspace("ws1");
    let ws1_root = repo.path().parent().unwrap().join("ws1");
    std::fs::write(ws1_root.join(".gitignore"), "secret*\n").unwrap();
    std::fs::write(ws1_root.join("secret.env"), "SECRET=1").unwrap();
    std::fs::write(ws1_root.join("plain.txt"), "not ignored").unwrap();

    let jj = CliJj::at(repo.path().clone());
    let report = copy_ignored::run(
        &jj,
        CopyIgnoredArgs {
            from: Some("ws1"),
            to: None, // current = the repo root workspace
            force: false,
        },
    )
    .expect("copy-ignored");

    let copied_names: Vec<_> = report
        .copied
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert!(copied_names.contains(&"secret.env".to_string()));
    assert!(
        !copied_names.contains(&"plain.txt".to_string()),
        "untracked-not-ignored must not move: {copied_names:?}"
    );
    assert_eq!(
        std::fs::read_to_string(repo.path().join("secret.env")).unwrap(),
        "SECRET=1"
    );
}

#[test]
fn honors_nested_gitignore_files() {
    let repo = TempRepo::init();
    repo.add_workspace("ws1");
    let ws1_root = repo.path().parent().unwrap().join("ws1");
    std::fs::create_dir_all(ws1_root.join("nested")).unwrap();
    std::fs::write(ws1_root.join("nested/.gitignore"), "cache*\n").unwrap();
    std::fs::write(ws1_root.join("nested/cache.db"), "bytes").unwrap();

    let jj = CliJj::at(repo.path().clone());
    let report = copy_ignored::run(
        &jj,
        CopyIgnoredArgs {
            from: Some("ws1"),
            to: None,
            force: false,
        },
    )
    .expect("copy-ignored");

    assert!(report.copied.iter().any(|p| p.ends_with("cache.db")));
}

#[test]
fn skips_existing_destination_files_without_force() {
    let repo = TempRepo::init();
    repo.add_workspace("ws1");
    let ws1_root = repo.path().parent().unwrap().join("ws1");
    std::fs::write(ws1_root.join(".gitignore"), "secret*\n").unwrap();
    std::fs::write(ws1_root.join("secret.env"), "FROM_WS1").unwrap();
    // The destination already has its own (different) copy.
    std::fs::write(repo.path().join("secret.env"), "ALREADY_HERE").unwrap();

    let jj = CliJj::at(repo.path().clone());
    let report = copy_ignored::run(
        &jj,
        CopyIgnoredArgs {
            from: Some("ws1"),
            to: None,
            force: false,
        },
    )
    .expect("copy-ignored without force");

    assert_eq!(report.skipped_existing.len(), 1);
    assert_eq!(
        std::fs::read_to_string(repo.path().join("secret.env")).unwrap(),
        "ALREADY_HERE"
    );

    let report = copy_ignored::run(
        &jj,
        CopyIgnoredArgs {
            from: Some("ws1"),
            to: None,
            force: true,
        },
    )
    .expect("copy-ignored with force");

    assert_eq!(report.copied.len(), 1);
    assert_eq!(
        std::fs::read_to_string(repo.path().join("secret.env")).unwrap(),
        "FROM_WS1"
    );
}

#[test]
fn worktreeinclude_filters_the_selection() {
    let repo = TempRepo::init();
    repo.add_workspace("ws1");
    let ws1_root = repo.path().parent().unwrap().join("ws1");
    std::fs::write(ws1_root.join(".gitignore"), "keep*\nshared*\n").unwrap();
    std::fs::write(ws1_root.join("keep.env"), "keep").unwrap();
    std::fs::write(ws1_root.join("shared.txt"), "shared").unwrap();
    // Only `keep*` files are wanted in other worktrees.
    std::fs::write(ws1_root.join(".worktreeinclude"), "keep*\n").unwrap();

    let jj = CliJj::at(repo.path().clone());
    let report = copy_ignored::run(
        &jj,
        CopyIgnoredArgs {
            from: Some("ws1"),
            to: None,
            force: false,
        },
    )
    .expect("copy-ignored");

    let copied_names: Vec<_> = report
        .copied
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert!(copied_names.contains(&"keep.env".to_string()));
    assert!(!copied_names.contains(&"shared.txt".to_string()));
}
