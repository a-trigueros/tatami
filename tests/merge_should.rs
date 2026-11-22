//! Proof tier: `tt merge` / `tt rebase` against a real jj repo.
//!
//! Real usage: the user stands in the workspace being merged, so every
//! invocation here runs with the adapter rooted in that workspace.

mod common;

use common::TempRepo;
use std::path::Path;
use tatami::cmd::merge::{self, MergeArgs};
use tatami::cmd::rebase;
use tatami::hook::{Hooks, NoGate};
use tatami::jj::{CliJj, Jj};

static NO_HOOKS: Hooks<'static> = Hooks {
    user: None,
    project: None,
};

fn merge_args<'a>(
    target: Option<&'a str>,
    remove_current: bool,
    caller_root: &'a Path,
    state_dir: &'a Path,
) -> MergeArgs<'a> {
    MergeArgs {
        target,
        remove_current,
        hooks: &NO_HOOKS,
        caller_root,
        logs_dir: None,
        state_dir,
        now_millis: 1_000,
    }
}

fn create_main(repo: &TempRepo) {
    repo.capture_jj(&["bookmark", "create", "main", "-r", "@"]);
}

/// A workspace with one named real commit (bookmark left to the caller).
fn workspace_with_commit(repo: &TempRepo, name: &str, message: &str) {
    repo.add_workspace(name);
    let root = repo.path().parent().unwrap().join(name);
    std::fs::write(root.join(format!("{name}.txt")), "content").unwrap();
    repo.capture_jj_in(&root, &["commit", "-m", message]);
    repo.capture_jj_in(
        &root,
        &["bookmark", "create", name, "-r", &format!("{name}@-")],
    );
}

#[test]
fn fast_forwards_when_the_workspace_is_descendant() {
    let repo = TempRepo::init();
    create_main(&repo);
    // Created ON the main bookmark — this workspace is a descendant of main.
    repo.capture_jj(&["workspace", "add", "-r", "main", "../ws1"]);
    let ws1_root = repo.path().parent().unwrap().join("ws1");
    std::fs::write(ws1_root.join("ws1.txt"), "content").unwrap();
    repo.capture_jj_in(&ws1_root, &["commit", "-m", "feature A"]);
    repo.capture_jj_in(&ws1_root, &["bookmark", "create", "ws1", "-r", "ws1@-"]);

    let jj = CliJj::at(ws1_root.clone());
    let outcome = merge::run(
        &jj,
        merge_args(None, false, &ws1_root, &ws1_root),
        &mut NoGate,
        &mut Vec::new(),
    )
    .expect("merge ws1");

    assert_eq!(outcome.target, "main");
    assert!(!outcome.rebased, "already a descendant: pure fast-forward");
    assert_eq!(outcome.commits, 1);
    assert!(outcome.bookmark_moved);
    let at_main = repo.capture_jj(&["log", "-r", "main", "--no-graph", "-T", "description"]);
    assert_eq!(at_main, "feature A");
}

#[test]
fn rebases_diverged_commits_then_moves_the_target() {
    let repo = TempRepo::init();
    create_main(&repo);
    workspace_with_commit(&repo, "ws1", "feature A");
    let ws1_root = repo.path().parent().unwrap().join("ws1");

    // Advance main independently: the chains diverge.
    std::fs::write(repo.path().join("main.txt"), "on main").unwrap();
    repo.capture_jj(&["commit", "-m", "main work"]);
    repo.capture_jj(&["bookmark", "set", "main", "-r", "@-"]);

    let jj = CliJj::at(ws1_root.clone());
    let outcome = merge::run(
        &jj,
        merge_args(None, false, &ws1_root, &ws1_root),
        &mut NoGate,
        &mut Vec::new(),
    )
    .expect("merge diverged ws1");

    assert!(outcome.rebased, "diverged chains need a rebase");
    let missing = repo.capture_jj(&[
        "log",
        "-r",
        "::ws1@ ~ ::main",
        "--no-graph",
        "-T",
        "description",
    ]);
    assert_eq!(missing, "", "commits left out of main: {missing:?}");
}

#[test]
fn rebase_reparents_without_moving_the_target() {
    let repo = TempRepo::init();
    create_main(&repo);
    workspace_with_commit(&repo, "ws1", "feature A");
    let ws1_root = repo.path().parent().unwrap().join("ws1");

    std::fs::write(repo.path().join("main.txt"), "on main").unwrap();
    repo.capture_jj(&["commit", "-m", "main work"]);
    repo.capture_jj(&["bookmark", "set", "main", "-r", "@-"]);

    let jj = CliJj::at(ws1_root.clone());
    let outcome = rebase::run(&jj, None).expect("rebase ws1");

    assert!(outcome.rebased);
    assert!(!outcome.bookmark_moved, "rebase never moves the target");
    let chain = repo.capture_jj(&["log", "-r", "main::ws1@", "--no-graph", "-T", "description"]);
    assert!(chain.contains("main work"), "chain was: {chain:?}");
    assert!(chain.contains("feature A"), "chain was: {chain:?}");
}

#[test]
fn requires_the_namesake_bookmark() {
    let repo = TempRepo::init();
    create_main(&repo);
    repo.add_workspace("loose");
    let loose_root = repo.path().parent().unwrap().join("loose");

    let jj = CliJj::at(loose_root.clone());
    let error = merge::run(
        &jj,
        merge_args(None, false, &loose_root, &loose_root),
        &mut NoGate,
        &mut Vec::new(),
    )
    .unwrap_err();

    assert!(
        matches!(error, merge::MergeError::RequireNamesakeBookmark { .. }),
        "got: {error:?}"
    );
}

#[test]
fn reports_nothing_to_merge_when_fully_integrated() {
    let repo = TempRepo::init();
    create_main(&repo);
    repo.add_workspace("ws1");
    repo.capture_jj(&["bookmark", "create", "ws1", "-r", "ws1@"]);
    let ws1_root = repo.path().parent().unwrap().join("ws1");

    let jj = CliJj::at(ws1_root.clone());
    let error = merge::run(
        &jj,
        merge_args(None, false, &ws1_root, &ws1_root),
        &mut NoGate,
        &mut Vec::new(),
    )
    .unwrap_err();

    assert!(
        matches!(error, merge::MergeError::NoCommitsToMerge { .. }),
        "got: {error:?}"
    );
}

#[test]
fn falls_back_to_master_when_main_is_absent() {
    let repo = TempRepo::init();
    repo.capture_jj(&["bookmark", "create", "master", "-r", "@"]);
    workspace_with_commit(&repo, "ws1", "feature A");
    let ws1_root = repo.path().parent().unwrap().join("ws1");

    let jj = CliJj::at(ws1_root.clone());
    let outcome = merge::run(
        &jj,
        merge_args(None, false, &ws1_root, &ws1_root),
        &mut NoGate,
        &mut Vec::new(),
    )
    .expect("merge onto master");

    assert_eq!(outcome.target, "master");
}

#[test]
fn an_explicit_target_wins_over_the_detection() {
    let repo = TempRepo::init();
    repo.capture_jj(&["bookmark", "create", "main", "-r", "@"]);
    repo.capture_jj(&["bookmark", "create", "release", "-r", "@"]);
    workspace_with_commit(&repo, "ws1", "feature A");
    let ws1_root = repo.path().parent().unwrap().join("ws1");

    let jj = CliJj::at(ws1_root.clone());
    let outcome = merge::run(
        &jj,
        merge_args(Some("release"), false, &ws1_root, &ws1_root),
        &mut NoGate,
        &mut Vec::new(),
    )
    .expect("merge onto release");

    assert_eq!(outcome.target, "release");
}

#[test]
fn a_failing_pre_merge_hook_aborts_before_any_change() {
    let repo = TempRepo::init();
    create_main(&repo);
    workspace_with_commit(&repo, "ws1", "feature A");
    let ws1_root = repo.path().parent().unwrap().join("ws1");

    let user: tatami::config::UserConfig = toml::from_str("pre-merge = \"exit 1\"").unwrap();
    let hooks = Hooks {
        user: Some(&user),
        project: None,
    };

    let jj = CliJj::at(ws1_root.clone());
    let error = merge::run(
        &jj,
        merge::MergeArgs {
            target: None,
            remove_current: false,
            hooks: &hooks,
            caller_root: &ws1_root,
            logs_dir: None,
            state_dir: &ws1_root,
            now_millis: 1_000,
        },
        &mut NoGate,
        &mut Vec::new(),
    )
    .unwrap_err();

    assert!(matches!(error, merge::MergeError::Hook(_)), "{error:?}");
    let at_main = repo.capture_jj(&["log", "-r", "main", "--no-graph", "-T", "description"]);
    assert_ne!(at_main, "feature A", "main must not have moved");
}

#[test]
fn remove_forgets_the_merged_workspace_and_lands_on_the_target() {
    let repo = TempRepo::init();
    create_main(&repo);
    workspace_with_commit(&repo, "ws1", "feature A");
    let ws1_root = repo.path().parent().unwrap().join("ws1");

    let jj = CliJj::at(ws1_root.clone());
    let trash_state = tempfile::tempdir().unwrap();
    let outcome = merge::run(
        &jj,
        merge_args(None, true, &ws1_root, trash_state.path()),
        &mut NoGate,
        &mut Vec::new(),
    )
    .expect("merge --remove ws1");

    assert_eq!(outcome.landing, Some(repo.path()));
    assert!(outcome.removed_current);
    // The adapter rooted in the (now removed) ws1 directory is dead; query
    // from the host workspace.
    let jj = CliJj::at(repo.path().clone());
    let names: Vec<_> = jj
        .workspaces()
        .unwrap()
        .into_iter()
        .map(|w| w.name)
        .collect();
    assert!(
        !names.contains(&"ws1".to_string()),
        "ws1 must be forgotten: {names:?}"
    );
    assert!(!ws1_root.exists(), "directory must be trashed");
    let at_main = repo.capture_jj(&["log", "-r", "main", "--no-graph", "-T", "description"]);
    assert_eq!(at_main, "feature A");
}

#[test]
fn remove_refuses_the_host_workspace_before_anything_happens() {
    let repo = TempRepo::init();
    create_main(&repo);
    let main_root = repo.path();

    let jj = CliJj::at(main_root.clone());
    let error = merge::run(
        &jj,
        merge_args(None, true, &main_root, &main_root),
        &mut NoGate,
        &mut Vec::new(),
    )
    .unwrap_err();

    assert!(
        matches!(error, merge::MergeError::RefuseHostWorkspace { .. }),
        "the host workspace must never be removed: {error:?}"
    );
}
