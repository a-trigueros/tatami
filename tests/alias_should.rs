//! Proof tier: `tt <alias>` against a real jj repo.

mod common;

use common::TempRepo;
use tatami::cmd::alias;
use tatami::config::schema::{ProjectConfig, UserConfig};
use tatami::hook::{ApprovalGate, NoGate};
use tatami::jj::CliJj;

struct Approving;
impl ApprovalGate for Approving {
    fn approve(&mut self, _key: &str, _command: &str) -> bool {
        true
    }
}

struct Denying;
impl ApprovalGate for Denying {
    fn approve(&mut self, _key: &str, _command: &str) -> bool {
        false
    }
}

#[test]
fn a_user_alias_runs_with_the_workspace_variables() {
    let repo = TempRepo::init();
    let user: UserConfig = toml::from_str(
        "aliases.describe-here = \"echo {{ workspace }}-{{ args }} > aliased.txt\"\n",
    )
    .unwrap();

    let jj = CliJj::at(repo.path().clone());
    alias::run(
        &jj,
        "describe-here",
        &["extra".to_string()],
        &user,
        &ProjectConfig::default(),
        &mut NoGate,
        &repo.path(),
    )
    .expect("alias runs");

    let output = std::fs::read_to_string(repo.path().join("aliased.txt")).unwrap();
    assert_eq!(output, "default-extra\n");
}

#[test]
fn user_then_project_aliases_accumulate_in_order() {
    let repo = TempRepo::init();
    let user: UserConfig = toml::from_str("aliases.order = \"echo user > order.txt\"\n").unwrap();
    let project: ProjectConfig =
        toml::from_str("aliases.order = \"echo project >> order.txt\"\n").unwrap();

    let jj = CliJj::at(repo.path().clone());
    alias::run(
        &jj,
        "order",
        &[],
        &user,
        &project,
        &mut Approving,
        &repo.path(),
    )
    .expect("both bodies run");

    let order = std::fs::read_to_string(repo.path().join("order.txt")).unwrap();
    assert_eq!(order, "user\nproject\n");
}

#[test]
fn an_unapproved_project_alias_is_denied() {
    let repo = TempRepo::init();
    let project: ProjectConfig =
        toml::from_str("aliases.greet = \"echo denied-but-ran > denied.txt\"\n").unwrap();

    let jj = CliJj::at(repo.path().clone());
    let error = alias::run(
        &jj,
        "greet",
        &[],
        &UserConfig::default(),
        &project,
        &mut Denying,
        &repo.path(),
    )
    .unwrap_err();

    assert!(
        matches!(error, alias::AliasError::Denied { .. }),
        "got: {error:?}"
    );
    assert!(
        !repo.path().join("denied.txt").exists(),
        "a denied alias must not run"
    );
}

#[test]
fn an_alias_failure_propagates() {
    let repo = TempRepo::init();
    let user: UserConfig = toml::from_str("aliases.boom = \"exit 3\"\n").unwrap();

    let jj = CliJj::at(repo.path().clone());
    let error = alias::run(
        &jj,
        "boom",
        &[],
        &user,
        &ProjectConfig::default(),
        &mut NoGate,
        &repo.path(),
    )
    .unwrap_err();

    assert!(
        matches!(error, alias::AliasError::Failed { status: 3, .. }),
        "got: {error:?}"
    );
}

#[test]
fn an_unknown_alias_is_a_clear_error() {
    let repo = TempRepo::init();

    let jj = CliJj::at(repo.path().clone());
    let error = alias::run(
        &jj,
        "nope",
        &[],
        &UserConfig::default(),
        &ProjectConfig::default(),
        &mut NoGate,
        &repo.path(),
    )
    .unwrap_err();

    assert!(
        matches!(error, alias::AliasError::UnknownAlias { .. }),
        "got: {error:?}"
    );
}
