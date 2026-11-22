//! Tatami config schema — strictly worktrunk-compatible (same keys, same
//! shapes), parsed leniently.
//!
//! Lenient means: unknown keys are ignored. A shared worktrunk config keeps
//! loading while tatami implements sections progressively; a typed model is
//! added when a milestone first consumes the section it models.

use std::collections::BTreeMap;

use serde::Deserialize;

/// A hook or alias value, in one of worktrunk's three shapes (PLAN §4):
/// one command string, named concurrent commands, or an ordered pipeline of
/// steps (concurrent within a step; a failed step stops the pipeline).
/// TOML cannot mix the table and array-of-tables shapes for one key, so a
/// given hook always has exactly one shape.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum CommandSpec {
    Single(String),
    Concurrent(BTreeMap<String, String>),
    Pipeline(Vec<BTreeMap<String, String>>),
}

/// The shared, checked-in project config (`.config/tatami.toml`, fallback
/// `.config/wt.toml`). Deliberately narrower than [`UserConfig`]: no
/// `workspace-path` — the repository does not dictate personal layout. What
/// it carries is what makes approvals necessary: shared hooks.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct ProjectConfig {
    #[serde(rename = "pre-switch")]
    pre_switch: Option<CommandSpec>,
    #[serde(rename = "post-switch")]
    post_switch: Option<CommandSpec>,
    #[serde(rename = "pre-start")]
    pre_start: Option<CommandSpec>,
    #[serde(rename = "post-start")]
    post_start: Option<CommandSpec>,
    #[serde(rename = "pre-remove")]
    pre_remove: Option<CommandSpec>,
    #[serde(rename = "post-remove")]
    post_remove: Option<CommandSpec>,
    #[serde(rename = "pre-merge")]
    pre_merge: Option<CommandSpec>,
    #[serde(rename = "post-merge")]
    post_merge: Option<CommandSpec>,
    #[serde(rename = "pre-commit")]
    pre_commit: Option<CommandSpec>,
    #[serde(rename = "post-commit")]
    post_commit: Option<CommandSpec>,
    /// Sections tatami does not execute yet (aliases, list, forge, step…).
    #[serde(flatten)]
    unparsed: BTreeMap<String, toml::Value>,
}

impl ProjectConfig {
    pub fn hook(&self, phase: crate::hook::HookPhase) -> Option<&CommandSpec> {
        match phase {
            crate::hook::HookPhase::PreSwitch => self.pre_switch.as_ref(),
            crate::hook::HookPhase::PostSwitch => self.post_switch.as_ref(),
            crate::hook::HookPhase::PreStart => self.pre_start.as_ref(),
            crate::hook::HookPhase::PostStart => self.post_start.as_ref(),
            crate::hook::HookPhase::PreRemove => self.pre_remove.as_ref(),
            crate::hook::HookPhase::PostRemove => self.post_remove.as_ref(),
            crate::hook::HookPhase::PreMerge => self.pre_merge.as_ref(),
            crate::hook::HookPhase::PostMerge => self.post_merge.as_ref(),
            crate::hook::HookPhase::PreCommit => self.pre_commit.as_ref(),
            crate::hook::HookPhase::PostCommit => self.post_commit.as_ref(),
        }
    }
}

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
    #[serde(rename = "pre-switch")]
    pre_switch: Option<CommandSpec>,
    #[serde(rename = "post-switch")]
    post_switch: Option<CommandSpec>,
    #[serde(rename = "pre-start")]
    pre_start: Option<CommandSpec>,
    #[serde(rename = "post-start")]
    post_start: Option<CommandSpec>,
    #[serde(rename = "pre-remove")]
    pre_remove: Option<CommandSpec>,
    #[serde(rename = "post-remove")]
    post_remove: Option<CommandSpec>,
    #[serde(rename = "pre-merge")]
    pre_merge: Option<CommandSpec>,
    #[serde(rename = "post-merge")]
    post_merge: Option<CommandSpec>,
    #[serde(rename = "pre-commit")]
    pre_commit: Option<CommandSpec>,
    #[serde(rename = "post-commit")]
    post_commit: Option<CommandSpec>,
    /// Sections tatami does not execute yet (aliases, switch, merge, remove,
    /// commit, step, list, projects, and hooks for commands tatami does not
    /// have yet like pre-commit). Kept for lossless round-trip introspection.
    /// shortcut: raw passthrough; each section gets a typed model in the
    /// milestone that first consumes it (M4 aliases).
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

    /// The hook configured for a lifecycle phase, if any.
    pub fn hook(&self, phase: crate::hook::HookPhase) -> Option<&CommandSpec> {
        match phase {
            crate::hook::HookPhase::PreSwitch => self.pre_switch.as_ref(),
            crate::hook::HookPhase::PostSwitch => self.post_switch.as_ref(),
            crate::hook::HookPhase::PreStart => self.pre_start.as_ref(),
            crate::hook::HookPhase::PostStart => self.post_start.as_ref(),
            crate::hook::HookPhase::PreRemove => self.pre_remove.as_ref(),
            crate::hook::HookPhase::PostRemove => self.post_remove.as_ref(),
            crate::hook::HookPhase::PreMerge => self.pre_merge.as_ref(),
            crate::hook::HookPhase::PostMerge => self.post_merge.as_ref(),
            crate::hook::HookPhase::PreCommit => self.pre_commit.as_ref(),
            crate::hook::HookPhase::PostCommit => self.post_commit.as_ref(),
        }
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

    #[test]
    fn parses_hook_shapes_for_the_phases_tatami_executes() {
        let config = parse(
            r#"
            pre-start = "npm install"

            [post-start]
            server = "npm run dev"

            [[pre-remove]]
            archive = "tar czf backup.tgz ."

            [[pre-remove]]
            report = "echo done"
            "#,
        );

        assert_eq!(
            config.hook(crate::hook::HookPhase::PreStart),
            Some(&CommandSpec::Single("npm install".to_string()))
        );
        assert_eq!(
            config.hook(crate::hook::HookPhase::PostStart),
            Some(&CommandSpec::Concurrent(BTreeMap::from([(
                "server".to_string(),
                "npm run dev".to_string()
            )])))
        );
        assert_eq!(
            config.hook(crate::hook::HookPhase::PreRemove),
            Some(&CommandSpec::Pipeline(vec![
                BTreeMap::from([("archive".to_string(), "tar czf backup.tgz .".to_string())]),
                BTreeMap::from([("report".to_string(), "echo done".to_string())]),
            ]))
        );
    }

    #[test]
    fn parses_the_full_hook_lifecycle_typed() {
        let config = parse("pre-commit = \"cargo fmt\"");

        assert_eq!(
            config.hook(crate::hook::HookPhase::PreCommit),
            Some(&CommandSpec::Single("cargo fmt".to_string()))
        );
        assert!(
            !config.unparsed.contains_key("pre-commit"),
            "pre-commit is now a typed hook: {:?}",
            config.unparsed.keys().collect::<Vec<_>>()
        );
    }
}
