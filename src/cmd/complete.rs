//! Dynamic shell-completion candidates, served to the completion scripts via
//! the hidden `tatami __complete <target>` command. The scripts (see
//! `config shell completion`) call it at completion time — candidate lists
//! live in the repo, not in a static script.

use thiserror::Error;

use crate::jj::{Jj, JjError};

/// Candidates for `<target>`: workspaces for `switch` (all) and `remove`
/// (the current one cannot be removed), bookmarks for `merge`/`rebase`
/// targets. Sorted for stable, readable completion menus. Command shortcuts
/// (`sw`, `rm`, `mg`, `rb`) normalize to their canonical target so the
/// completion scripts can pass the typed word through unchanged.
pub fn run(jj: &dyn Jj, target: &str) -> Result<Vec<String>, CompleteError> {
    let mut names = match target {
        "switch" | "sw" => crate::cmd::with_stale_recovery(jj, |jj| jj.workspaces())?
            .into_iter()
            .map(|w| w.name)
            .collect(),
        "remove" | "rm" => crate::cmd::with_stale_recovery(jj, |jj| jj.workspaces())?
            .into_iter()
            .filter(|w| !w.is_current)
            .map(|w| w.name)
            .collect(),
        "merge" | "mg" | "rebase" | "rb" => jj.bookmark_names()?,
        _ => Vec::new(),
    };
    names.sort();
    Ok(names)
}

#[derive(Debug, Error)]
pub enum CompleteError {
    #[error(transparent)]
    Jj(#[from] JjError),
    #[error(transparent)]
    State(#[from] crate::state::StateError),
}

#[cfg(test)]
mod complete_should {
    use super::*;
    use crate::jj::WorkspaceInfo;
    use std::cell::RefCell;

    struct FakeJj {
        workspaces: Vec<WorkspaceInfo>,
        bookmarks: Vec<String>,
        calls: RefCell<Vec<String>>,
    }
    impl FakeJj {
        fn new(workspaces: Vec<WorkspaceInfo>, bookmarks: Vec<String>) -> Self {
            Self {
                workspaces,
                bookmarks,
                calls: RefCell::new(Vec::new()),
            }
        }
    }
    impl Jj for FakeJj {
        fn workspace_root(&self) -> Result<std::path::PathBuf, JjError> {
            unimplemented!()
        }
        fn workspaces(&self) -> Result<Vec<WorkspaceInfo>, JjError> {
            self.calls.borrow_mut().push("workspaces".to_string());
            Ok(self.workspaces.clone())
        }
        fn workspace_add(&self, _: &str, _: &std::path::Path) -> Result<(), JjError> {
            unimplemented!()
        }
        fn bookmark_create(&self, _: &str, _: &str) -> Result<(), JjError> {
            unimplemented!()
        }
        fn workspace_forget(&self, _: &str) -> Result<(), JjError> {
            unimplemented!()
        }
        fn abandon(&self, _: &str) -> Result<(), JjError> {
            unimplemented!()
        }
        fn bookmark_delete(&self, _: &str) -> Result<bool, JjError> {
            unimplemented!()
        }
        fn bookmark_names(&self) -> Result<Vec<String>, JjError> {
            self.calls.borrow_mut().push("bookmarks".to_string());
            Ok(self.bookmarks.clone())
        }
        fn bookmark_commit_id(&self, _: &str) -> Result<String, JjError> {
            unimplemented!()
        }
        fn revset_count(&self, _: &str) -> Result<usize, JjError> {
            unimplemented!()
        }
        fn revset_commit_ids(&self, _: &str) -> Result<Vec<String>, JjError> {
            unimplemented!()
        }
        fn rebase(&self, _: &str, _: &str) -> Result<(), JjError> {
            unimplemented!()
        }
        fn bookmark_set(&self, _: &str, _: &str) -> Result<(), JjError> {
            unimplemented!()
        }
        fn workspace_update_stale(&self) -> Result<(), JjError> {
            unimplemented!()
        }
        fn commit(&self, _: &str) -> Result<(), JjError> {
            unimplemented!()
        }
        fn squash(&self) -> Result<(), JjError> {
            unimplemented!()
        }
        fn push_bookmark(&self, _: &str) -> Result<(), JjError> {
            unimplemented!()
        }
        fn diff(&self) -> Result<String, JjError> {
            unimplemented!()
        }
    }

    fn ws(name: &str, current: bool) -> WorkspaceInfo {
        WorkspaceInfo {
            name: name.to_string(),
            root: None,
            is_current: current,
            wc: crate::jj::WcStatus {
                change: String::new(),
                description: String::new(),
                empty: true,
                conflict: false,
                divergent: false,
            },
        }
    }

    #[test]
    fn switch_offers_every_workspace_sorted() {
        let jj = FakeJj::new(
            vec![ws("zeta", false), ws("default", true), ws("alpha", false)],
            vec![],
        );
        assert_eq!(
            run(&jj, "switch").unwrap(),
            vec!["alpha", "default", "zeta"]
        );
    }

    #[test]
    fn remove_excludes_the_current_workspace() {
        let jj = FakeJj::new(
            vec![ws("zeta", false), ws("default", true), ws("alpha", false)],
            vec![],
        );
        assert_eq!(run(&jj, "remove").unwrap(), vec!["alpha", "zeta"]);
    }

    #[test]
    fn merge_and_rebase_offer_bookmarks_sorted() {
        let jj = FakeJj::new(
            vec![],
            vec!["zeta".to_string(), "main".into(), "alpha".into()],
        );
        assert_eq!(run(&jj, "merge").unwrap(), vec!["alpha", "main", "zeta"]);
        assert_eq!(run(&jj, "rebase").unwrap(), vec!["alpha", "main", "zeta"]);
        assert!(matches!(
            jj.calls.borrow().as_slice(),
            [b, _] if b == "bookmarks"
        ));
    }

    #[test]
    fn unknown_targets_offer_nothing() {
        let jj = FakeJj::new(vec![], vec![]);
        assert!(run(&jj, "list").unwrap().is_empty());
    }

    #[test]
    fn shortcuts_complete_like_their_canonical_target() {
        let jj = FakeJj::new(
            vec![ws("zeta", false), ws("default", true), ws("alpha", false)],
            vec!["zeta".to_string(), "main".into(), "alpha".into()],
        );
        assert_eq!(run(&jj, "sw").unwrap(), vec!["alpha", "default", "zeta"]);
        assert_eq!(run(&jj, "rm").unwrap(), vec!["alpha", "zeta"]);
        assert_eq!(run(&jj, "mg").unwrap(), vec!["alpha", "main", "zeta"]);
        assert_eq!(run(&jj, "rb").unwrap(), vec!["alpha", "main", "zeta"]);
    }
}
