//! Tatami config schema — strictly worktrunk-compatible (same keys, same
//! shapes), parsed leniently.
//!
//! Lenient means: unknown keys are ignored. A shared worktrunk config keeps
//! loading while tatami implements sections progressively; a typed model is
//! added when a milestone first consumes the section it models.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The user config file's content.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct UserConfig {
    /// Where tatami creates workspaces (template, expanded later).
    #[serde(rename = "workspace-path")]
    workspace_path: Option<String>,
    /// Legacy worktrunk key, honored when `workspace-path` is absent.
    #[serde(rename = "worktree-path")]
    worktree_path: Option<String>,
    /// Sections tatami does not execute yet (hooks, aliases, switch, merge,
    /// remove, commit, step, list, projects). Kept for lossless round-trip
    /// introspection.
    /// shortcut: raw passthrough; each section gets a typed model in the
    /// milestone that first consumes it (M2 hooks, M4 aliases).
    #[serde(flatten)]
    unparsed: BTreeMap<String, toml::Value>,
}

impl UserConfig {
    /// Workspace location template: `workspace-path`, else legacy
    /// `worktree-path`. Returned raw — variable expansion is a later
    /// milestone's job (M4), never the loader's.
    pub fn workspace_path(&self) -> Option<&str> {
        self.workspace_path
            .as_deref()
            .or(self.worktree_path.as_deref())
    }
}

#[cfg(test)]
mod workspace_path_should {
    use super::*;
    use pretty_assertions::assert_str_eq;

    fn parse(raw: &str) -> UserConfig {
        toml::from_str(raw).unwrap()
    }

    #[test]
    fn prefers_workspace_path_over_the_legacy_worktree_path() {
        let config = parse(
            "workspace-path = \".ws/{{ workspace }}\"\nworktree-path = \".wt/{{ branch }}\"\n",
        );
        assert_str_eq!(config.workspace_path().unwrap(), ".ws/{{ workspace }}");
    }

    #[test]
    fn uses_the_legacy_worktree_path_when_workspace_path_is_absent() {
        let config = parse("worktree-path = \".wt/{{ branch }}\"\n");
        assert_str_eq!(config.workspace_path().unwrap(), ".wt/{{ branch }}");
    }

    #[test]
    fn is_none_when_neither_key_is_set() {
        assert_eq!(parse("").workspace_path(), None);
    }

    #[test]
    fn ignores_unknown_keys_so_worktrunk_configs_keep_loading() {
        // A realistic worktrunk user config: top-level kebab-case hooks,
        // concurrent tables, pipelines, aliases, per-project overrides.
        let config = parse(
            r#"
            worktree-path = ".worktrees/{{ branch | sanitize }}"

            pre-start = "npm install"

            [post-start]
            server = "npm run dev"

            [[post-commit]]
            install = "npm ci"

            [[post-commit]]
            build = "npm run build"

            [aliases]
            greet = "echo Hello from {{ branch }}"

            [switch]
            cd = true

            [merge]
            squash = true

            [projects."github.com/owner/repo"]
            worktree-path = ".worktrees/{{ branch | sanitize }}"
            "#,
        );
        assert_str_eq!(
            config.workspace_path().unwrap(),
            ".worktrees/{{ branch | sanitize }}"
        );
    }
}
