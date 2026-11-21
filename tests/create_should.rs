//! Proof tier: `tt switch --create` against a real jj repo.

mod common;

use common::TempRepo;
use std::path::Path;
use tatami::cmd::create;
use tatami::jj::{CliJj, Jj};

#[test]
fn creates_a_workspace_and_its_namesake_bookmark() {
    let repo = TempRepo::init();
    repo.describe_working_copy("main workspace");

    let jj = CliJj::at(repo.path().clone());
    let dest = create::run(&jj, "feature", None, &repo.path(), Path::new("/home/u"))
        .expect("create feature");

    // Worktrunk-parity default: a sibling of the repo dir named
    // `<repo>.<workspace>` (here the repo dir is literally `repo`).
    assert_eq!(dest, repo.path().parent().unwrap().join("repo.feature"));
}

#[test]
fn the_created_workspace_is_listed_with_a_bookmark() {
    let repo = TempRepo::init();

    let jj = CliJj::at(repo.path().clone());
    let dest = create::run(&jj, "feature", None, &repo.path(), Path::new("/home/u")).unwrap();

    let workspaces = jj.workspaces().unwrap();
    let feature = workspaces
        .iter()
        .find(|w| w.name == "feature")
        .expect("created workspace listed");
    assert_eq!(feature.root, Some(dest));

    // The namesake bookmark points at the new workspace's @.
    let bookmarks = repo.capture_jj(&["bookmark", "list"]);
    assert!(
        bookmarks.starts_with("feature:"),
        "bookmark list was: {bookmarks:?}"
    );
}

#[test]
fn an_in_repo_template_does_not_pollute_the_main_snapshot() {
    let repo = TempRepo::init();

    let jj = CliJj::at(repo.path().clone());
    let dest = create::run(
        &jj,
        "inner",
        Some(".wt/{{ workspace }}"),
        &repo.path(),
        Path::new("/home/u"),
    )
    .expect("create with in-repo template");
    assert_eq!(dest, repo.path().join(".wt/inner"));

    // jj auto-excludes nested workspaces (verified on 0.47.1): the main
    // working copy must stay clean.
    let status = repo.capture_jj(&["st"]);
    assert!(
        status.contains("no changes"),
        "main working copy polluted: {status:?}"
    );
}
