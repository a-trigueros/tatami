//! Proof tier: `tt switch --create` against a real jj repo.

mod common;

use common::TempRepo;
use std::fs;
use std::path::Path;
use tatami::cmd::create;
use tatami::config::schema::UserConfig;
use tatami::hook::{Hooks, NoGate};
use tatami::jj::{CliJj, Jj};

fn hooks_for(config: &UserConfig) -> Hooks<'_> {
    Hooks {
        user: Some(config),
        project: None,
    }
}

fn write_user_config(home: &Path, content: &str) -> UserConfig {
    let dir = home.join(".config/tatami");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("config.toml"), content).unwrap();
    toml::from_str(content).unwrap()
}

#[test]
fn creates_a_workspace_and_its_namesake_bookmark() {
    let repo = TempRepo::init();
    repo.describe_working_copy("main workspace");

    let jj = CliJj::at(repo.path().clone());
    let dest = create::run(
        &jj,
        "feature",
        None,
        &repo.path(),
        Path::new("/home/u"),
        &Hooks::none(),
        &mut NoGate,
    )
    .expect("create feature");

    // Worktrunk-parity default: a sibling of the repo dir named
    // `<repo>.<workspace>` (here the repo dir is literally `repo`).
    assert_eq!(dest, repo.path().parent().unwrap().join("repo.feature"));
}

#[test]
fn the_created_workspace_is_listed_with_a_bookmark() {
    let repo = TempRepo::init();

    let jj = CliJj::at(repo.path().clone());
    let dest = create::run(
        &jj,
        "feature",
        None,
        &repo.path(),
        Path::new("/home/u"),
        &Hooks::none(),
        &mut NoGate,
    )
    .unwrap();

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
        &Hooks::none(),
        &mut NoGate,
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

#[test]
fn runs_the_start_hooks_in_the_new_workspace() {
    let repo = TempRepo::init();
    let home = tempfile::tempdir().unwrap();
    let config = write_user_config(
        home.path(),
        "pre-start = \"echo {{ workspace }} > pre.txt\"\n",
    );

    let jj = CliJj::at(repo.path().clone());
    let dest = create::run(
        &jj,
        "feature",
        None,
        &repo.path(),
        Path::new("/home/u"),
        &hooks_for(&config),
        &mut NoGate,
    )
    .expect("create with hooks");

    // pre-start ran inside the fresh workspace (its cwd), after creation.
    let pre = fs::read_to_string(dest.join("pre.txt")).unwrap();
    assert_eq!(pre, "feature\n");
}

#[test]
fn a_failing_pre_start_rolls_back_the_fresh_workspace() {
    let repo = TempRepo::init();
    let home = tempfile::tempdir().unwrap();
    let config = write_user_config(home.path(), "pre-start = \"exit 1\"\n");

    let jj = CliJj::at(repo.path().clone());
    let error = create::run(
        &jj,
        "feature",
        None,
        &repo.path(),
        Path::new("/home/u"),
        &hooks_for(&config),
        &mut NoGate,
    )
    .expect_err("pre-start failure must abort");

    assert!(matches!(error, create::CreateError::Hook(_)), "{error:?}");

    // Rollback: forgotten in jj, directory gone.
    let names: Vec<_> = jj
        .workspaces()
        .unwrap()
        .into_iter()
        .map(|w| w.name)
        .collect();
    assert!(
        !names.contains(&"feature".to_string()),
        "workspaces: {names:?}"
    );
    assert!(!repo.path().parent().unwrap().join("repo.feature").exists());
}
