//! Proof tier: `tt merge` / `tt rebase` against a real jj repo.
//!
//! Real usage: the user stands in the workspace being merged, so every
//! invocation here runs with the adapter rooted in that workspace.

mod common;

use common::TempRepo;
use tatami::cmd::{merge, rebase};
use tatami::hook::{Hooks, NoGate};
use tatami::jj::CliJj;

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
    // Created ON the main bookmark (`jj workspace add -r main`), unlike the
    // default sibling layout — this workspace is a descendant of main.
    repo.capture_jj(&["workspace", "add", "-r", "main", "../ws1"]);
    let ws1_root = repo.path().parent().unwrap().join("ws1");
    std::fs::write(ws1_root.join("ws1.txt"), "content").unwrap();
    repo.capture_jj_in(&ws1_root, &["commit", "-m", "feature A"]);
    repo.capture_jj_in(&ws1_root, &["bookmark", "create", "ws1", "-r", "ws1@-"]);

    let jj = CliJj::at(ws1_root.clone());
    let outcome = merge::run(
        &jj,
        None,
        &Hooks::none(),
        &mut NoGate,
        &ws1_root,
        &mut Vec::new(),
    )
    .expect("merge ws1");

    assert_eq!(outcome.target, "main");
    assert!(!outcome.rebased, "already a descendant: pure fast-forward");
    assert_eq!(outcome.commits, 1);
    assert!(outcome.bookmark_moved);

    // main now sits on the workspace's non-empty tip (never the empty @).
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
        None,
        &Hooks::none(),
        &mut NoGate,
        &ws1_root,
        &mut Vec::new(),
    )
    .expect("merge diverged ws1");

    assert!(outcome.rebased, "diverged chains need a rebase");
    // Linear history: everything reachable from the workspace is reachable
    // from main.
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
    let at_main = repo.capture_jj(&["log", "-r", "main", "--no-graph", "-T", "description"]);
    assert_eq!(at_main, "main work", "main bookmark must stay put");
    // The chain must now be rooted on main (descendant), without being
    // integrated: main stays put, and the whole workspace chain (main work,
    // feature A, empty tip) sits below ws1@.
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
        None,
        &Hooks::none(),
        &mut NoGate,
        &loose_root,
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
    // ws1 exists with a bookmark but no commits of its own.
    repo.add_workspace("ws1");
    repo.capture_jj(&["bookmark", "create", "ws1", "-r", "ws1@"]);
    let ws1_root = repo.path().parent().unwrap().join("ws1");

    let jj = CliJj::at(ws1_root.clone());
    let error = merge::run(
        &jj,
        None,
        &Hooks::none(),
        &mut NoGate,
        &ws1_root,
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
        None,
        &Hooks::none(),
        &mut NoGate,
        &ws1_root,
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
        Some("release"),
        &Hooks::none(),
        &mut NoGate,
        &ws1_root,
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
    let error = merge::run(&jj, None, &hooks, &mut NoGate, &ws1_root, &mut Vec::new()).unwrap_err();

    assert!(matches!(error, merge::MergeError::Hook(_)), "{error:?}");
    let at_main = repo.capture_jj(&["log", "-r", "main", "--no-graph", "-T", "description"]);
    assert_ne!(at_main, "feature A", "main must not have moved");
}
