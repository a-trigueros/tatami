//! Proof tier: `tt step …` against a real jj repo.

mod common;

use common::TempRepo;
use tatami::cmd::merge::MergeError;
use tatami::cmd::step;
use tatami::config::schema::UserConfig;
use tatami::hook::{Hooks, NoGate};
use tatami::jj::{CliJj, Jj};

fn hooks_for(config: &UserConfig) -> Hooks<'_> {
    Hooks {
        user: Some(config),
        project: None,
    }
}

fn parse_config(raw: &str) -> UserConfig {
    toml::from_str(raw).unwrap()
}

#[test]
fn commit_describes_the_working_copy_and_advances() {
    let repo = TempRepo::init();
    let jj = CliJj::at(repo.path().clone());
    std::fs::write(repo.path().join("a.txt"), "content").unwrap();

    step::commit(
        &jj,
        "add a.txt",
        &Hooks::none(),
        &mut NoGate,
        &repo.path(),
        None,
        &mut Vec::new(),
    )
    .expect("step commit");

    // The parent of @ carries the change and the description; @ is a fresh
    // empty child.
    let parent = repo.capture_jj(&["log", "-r", "@-", "--no-graph", "-T", "description"]);
    assert_eq!(parent, "add a.txt");
    let head = repo.capture_jj(&["log", "-r", "@", "--no-graph", "-T", "empty"]);
    assert_eq!(head, "true");
}

#[test]
fn a_failing_pre_commit_hook_aborts_the_commit() {
    let repo = TempRepo::init();
    let jj = CliJj::at(repo.path().clone());
    std::fs::write(repo.path().join("a.txt"), "content").unwrap();
    let user = parse_config("pre-commit = \"exit 1\"");
    let hooks = hooks_for(&user);

    let error = step::commit(
        &jj,
        "add a.txt",
        &hooks,
        &mut NoGate,
        &repo.path(),
        None,
        &mut Vec::new(),
    )
    .unwrap_err();

    assert!(matches!(error, step::StepError::Hook(_)), "{error:?}");
    let head = repo.capture_jj(&["log", "-r", "@", "--no-graph", "-T", "description"]);
    assert_eq!(head, "", "the working copy must stay undescribed");
}

#[test]
fn squash_moves_the_changes_into_the_parent() {
    let repo = TempRepo::init();
    let jj = CliJj::at(repo.path().clone());
    // A real parent first: squashing into the immutable root is refused.
    std::fs::write(repo.path().join("a.txt"), "content").unwrap();
    repo.capture_jj(&["commit", "-m", "add a.txt"]);
    std::fs::write(repo.path().join("b.txt"), "content").unwrap();

    step::squash(&jj).expect("step squash");

    let parent_files = repo.capture_jj(&["diff", "-r", "@-", "--summary"]);
    assert!(parent_files.contains("b.txt"), "files: {parent_files:?}");
}

#[test]
fn push_surfaces_jjs_error_when_no_remote_exists() {
    let repo = TempRepo::init();
    let jj = CliJj::at(repo.path().clone());
    repo.capture_jj(&["bookmark", "create", "default", "-r", "@"]);

    let error = step::push(&jj, Some("default")).unwrap_err();

    assert!(
        matches!(error, step::StepError::Jj(_)),
        "jj's own diagnosis must surface: {error:?}"
    );
}

#[test]
fn diff_shows_the_working_copy_changes() {
    let repo = TempRepo::init();
    let jj = CliJj::at(repo.path().clone());
    std::fs::write(repo.path().join("a.txt"), "content").unwrap();

    let diff = step::diff(&jj).expect("step diff");

    assert!(diff.contains("a.txt"), "diff was: {diff:?}");
}

#[test]
fn for_each_runs_in_every_existing_workspace_and_skips_missing_ones() {
    let repo = TempRepo::init();
    repo.add_workspace("ws1");
    repo.add_workspace("gone");
    let gone_root = repo.path().parent().unwrap().join("gone");
    std::fs::remove_dir_all(gone_root).unwrap();

    let jj = CliJj::at(repo.path().clone());
    let mut out = Vec::new();
    let outcomes = step::for_each(&jj, &["pwd".to_string()], &mut out).expect("for-each");

    let names: Vec<_> = outcomes.iter().map(|o| o.workspace.as_str()).collect();
    assert!(names.contains(&"default"), "outcomes: {outcomes:?}");
    assert!(names.contains(&"ws1"), "outcomes: {outcomes:?}");
    assert!(
        !names.contains(&"gone"),
        "missing directory must be skipped"
    );
    let notes = String::from_utf8_lossy(&out);
    assert!(notes.contains("gone"), "skip note missing: {notes:?}");
    assert!(
        outcomes.iter().all(|o| o.failed.is_none()),
        "pwd must succeed everywhere: {outcomes:?}"
    );
}

#[test]
fn prune_removes_only_fully_integrated_workspaces() {
    let repo = TempRepo::init();
    repo.capture_jj(&["bookmark", "create", "main", "-r", "@"]);
    // ws1: one merged commit, one pending → NOT pruned.
    repo.capture_jj(&["workspace", "add", "-r", "main", "../ws1"]);
    let ws1_root = repo.path().parent().unwrap().join("ws1");
    std::fs::write(ws1_root.join("merged.txt"), "x").unwrap();
    repo.capture_jj_in(&ws1_root, &["commit", "-m", "merged work"]);
    // Identify the merged commit structurally: it is the grandparent of the
    // workspace tip (merged -> pending -> empty @).
    repo.capture_jj(&["bookmark", "set", "main", "-r", "ws1@-"]);
    std::fs::write(ws1_root.join("pending.txt"), "x").unwrap();
    repo.capture_jj_in(&ws1_root, &["commit", "-m", "pending work"]);
    repo.capture_jj(&["bookmark", "create", "ws1", "-r", "ws1@-"]);
    // ws2: nothing outside main → pruned.
    repo.add_workspace("ws2");
    repo.capture_jj(&["bookmark", "create", "ws2", "-r", "ws2@"]);

    let jj = CliJj::at(repo.path().clone());
    let state_dir = tempfile::tempdir().unwrap();
    let pruned = step::prune(
        &jj,
        &Hooks::none(),
        &mut NoGate,
        &state_dir.path().join(".jj/tatami"),
        &repo.path(),
        1_000,
        &mut Vec::new(),
    )
    .expect("prune");

    let names: Vec<_> = pruned.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["ws2"]);
    let remaining: Vec<_> = jj
        .workspaces()
        .unwrap()
        .into_iter()
        .map(|w| w.name)
        .collect();
    assert!(remaining.contains(&"ws1".to_string()), "ws1 must survive");
    assert!(!remaining.contains(&"ws2".to_string()));
}

#[test]
fn prune_needs_a_default_target() {
    let repo = TempRepo::init();
    repo.add_workspace("ws1");
    repo.capture_jj(&["bookmark", "create", "ws1", "-r", "ws1@"]);

    let jj = CliJj::at(repo.path().clone());
    let state_dir = tempfile::tempdir().unwrap();
    let error = step::prune(
        &jj,
        &Hooks::none(),
        &mut NoGate,
        &state_dir.path().join(".jj/tatami"),
        &repo.path(),
        1_000,
        &mut Vec::new(),
    )
    .unwrap_err();

    assert!(
        matches!(
            error,
            step::StepError::Merge(MergeError::NoDefaultBookmark { .. })
                | step::StepError::Merge(MergeError::NoCommitsToMerge { .. })
        ),
        "no usable target must surface as an error: {error:?}"
    );
}

#[test]
fn for_each_rejects_an_empty_command() {
    let repo = TempRepo::init();
    let jj = CliJj::at(repo.path().clone());

    let error = step::for_each(&jj, &[], &mut Vec::new()).unwrap_err();

    assert!(matches!(error, step::StepError::EmptyCommand));
}
