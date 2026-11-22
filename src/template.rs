//! Template rendering (minijinja) for `workspace-path`, hooks and aliases.
//!
//! Worktrunk-parity facts (sourced 2026-11-22):
//! - the `sanitize` filter replaces **only** `/` and `\` with `-` — no
//!   lowercasing, no collapsing of repeats;
//! - `sanitize_db` keeps ASCII alphanumerics (lowercased), collapses every
//!   run of other characters into one `_`, prefixes `_` when the result
//!   starts with a digit, truncates to 44 characters and appends a 3-char
//!   base-36 hash of the *original* input (48 max).
//!   shortcut: the hash algorithm is tatami's own (FNV-1a), not bit-identical
//!   to worktrunk's — same guarantees, different suffixes.
//! - `codename(n)` is not implemented yet: rendering a template that uses it
//!   fails with a clear error.

use std::collections::BTreeMap;

use minijinja::{Environment, Value};
use thiserror::Error;

/// Template variables, keyed by name.
#[derive(Debug, Default, Clone)]
pub struct TemplateVars(pub BTreeMap<String, String>);

impl TemplateVars {
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub fn insert(&mut self, name: &str, value: String) -> &mut Self {
        self.0.insert(name.to_string(), value);
        self
    }
}

#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("cannot render template `{template}`: {source}")]
    Render {
        template: String,
        #[source]
        source: minijinja::Error,
    },
    #[error("template uses the filter `{filter}`, which tatami does not implement yet")]
    UnsupportedFilter { filter: String },
}

/// Renders `template` with `vars`. Missing variables render as empty
/// (lenient — worktrunk templates reference variables tatami does not
/// provide yet, like `owner` or `remote_repo`).
pub fn render(template: &str, vars: &TemplateVars) -> Result<String, TemplateError> {
    let mut environment = Environment::new();
    environment.set_undefined_behavior(minijinja::UndefinedBehavior::Lenient);
    environment.add_filter("sanitize", sanitize_filter);
    environment.add_filter("sanitize_db", sanitize_db_filter);
    environment.add_filter(
        "codename",
        |_value: Value, _count: Option<u64>| -> Result<String, minijinja::Error> {
            Err(minijinja::Error::new(
                minijinja::ErrorKind::InvalidOperation,
                "the codename filter is not implemented in tatami yet",
            ))
        },
    );
    environment.render_str(template, &vars.0).map_err(|source| {
        if source.to_string().contains("codename") {
            TemplateError::UnsupportedFilter {
                filter: "codename".to_string(),
            }
        } else {
            TemplateError::Render {
                template: template.to_string(),
                source,
            }
        }
    })
}

/// Worktrunk parity: only path separators become `-`.
fn sanitize_filter(value: String) -> String {
    value.replace(['/', '\\'], "-")
}

/// Worktrunk-parity shape; the hash suffix is tatami's own FNV-1a base-36.
fn sanitize_db_filter(value: String) -> String {
    if value.is_empty() {
        return String::new();
    }
    let mut base = String::new();
    let mut previous_was_separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            base.push(character.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !previous_was_separator {
            base.push('_');
            previous_was_separator = true;
        }
    }
    if base.starts_with(|c: char| c.is_ascii_digit()) {
        base.insert(0, '_');
    }
    base.truncate(44);
    let suffix = format!("{:07}", hash_base36(&value));
    let suffix = &suffix[suffix.len().saturating_sub(3)..];
    if base.ends_with('_') || base.is_empty() {
        format!("{base}{suffix}")
    } else {
        format!("{base}_{suffix}")
    }
}

/// FNV-1a 64-bit, rendered in base 36.
fn hash_base36(value: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    let mut digits = Vec::new();
    while hash > 0 {
        let digit = (hash % 36) as u8;
        digits.push(std::char::from_digit(u32::from(digit), 36).unwrap_or('0'));
        hash /= 36;
    }
    digits.into_iter().rev().collect()
}

#[cfg(test)]
mod render_should {
    use super::*;
    use pretty_assertions::assert_eq;

    fn render_with(template: &str, pairs: &[(&str, &str)]) -> String {
        let mut vars = TemplateVars::new();
        for (name, value) in pairs {
            vars.insert(name, (*value).to_string());
        }
        render(template, &vars).unwrap()
    }

    #[test]
    fn renders_the_workspace_variables() {
        assert_eq!(
            render_with(
                "{{ workspace }} at {{ workspace_path }}",
                &[("workspace", "feat"), ("workspace_path", "/repo/.wt/feat")]
            ),
            "feat at /repo/.wt/feat"
        );
    }

    #[test]
    fn renders_missing_variables_as_empty() {
        assert_eq!(render_with("[{{ owner }}]", &[]), "[]");
    }

    #[test]
    fn sanitize_replaces_only_path_separators() {
        assert_eq!(
            render_with("{{ x | sanitize }}", &[("x", "a/b\\c:d e")]),
            "a-b-c:d e"
        );
        assert_eq!(render_with("{{ x | sanitize }}", &[("x", "///")]), "---");
    }

    #[test]
    fn sanitize_db_lowercases_collapses_and_hashes() {
        let rendered = render_with("{{ x | sanitize_db }}", &[("x", "Feature/Login!!")]);
        assert!(rendered.starts_with("feature_login"), "was: {rendered:?}");
        assert!(rendered.len() <= 48, "was: {rendered:?}");
        // Deterministic for the same input.
        assert_eq!(
            rendered,
            render_with("{{ x | sanitize_db }}", &[("x", "Feature/Login!!")])
        );
    }

    #[test]
    fn sanitize_db_prefixes_an_underscore_before_a_leading_digit() {
        let rendered = render_with("{{ x | sanitize_db }}", &[("x", "42")]);
        assert!(rendered.starts_with('_'), "was: {rendered:?}");
    }

    #[test]
    fn fails_with_a_clear_error_on_the_unimplemented_codename_filter() {
        let error = render("{{ x | codename }}", &TemplateVars::new()).unwrap_err();
        assert!(
            matches!(error, TemplateError::UnsupportedFilter { .. }),
            "got: {error:?}"
        );
    }
}
