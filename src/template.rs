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

use minijinja::Environment;
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
        |value: String, count: Option<u64>| -> Result<String, minijinja::Error> {
            codename(&value, count).map_err(|message| {
                minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, message)
            })
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

/// `codename` — deterministic petname from the input (worktrunk parity of
/// intent: same medium word lists from the `petname` crate, `n - 1`
/// adjectives then a noun, hyphen-joined; repeated adjectives avoided).
/// shortcut: the word *selection* hash is tatami's FNV-1a, not bit-identical
/// to worktrunk's — same guarantees, different names.
fn codename(value: &str, count: Option<u64>) -> Result<String, String> {
    let count = count.unwrap_or(2).clamp(1, 8);
    let petnames = petname::lang::english::Petnames::medium();
    let adjectives = petnames.adjectives.as_ref();
    let nouns = petnames.nouns.as_ref();
    if adjectives.is_empty() || nouns.is_empty() {
        return Err("petname word lists are empty".to_string());
    }

    let mut hash = fnv1a(value);
    let mut words: Vec<&str> = Vec::with_capacity(count as usize);
    let mut used_adjectives: Vec<usize> = Vec::new();
    for _ in 0..count.saturating_sub(1) {
        let mut index = (hash as usize) % adjectives.len();
        // Avoid repeats where possible: advance until an unused word shows up
        // (bounded by the list length).
        for _ in 0..adjectives.len() {
            if !used_adjectives.contains(&index) {
                break;
            }
            index = (index + 1) % adjectives.len();
        }
        used_adjectives.push(index);
        words.push(adjectives[index]);
        hash = hash.wrapping_mul(0x100_0000_01b3).wrapping_add(1);
    }
    let noun_index = (hash as usize) % nouns.len();
    words.push(nouns[noun_index]);
    Ok(words.join("-"))
}

fn fnv1a(value: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
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
        // codename est implémenté; ce test vérifie plutôt le contrat.
        let first = render("{{ x | codename }}", &vars_x());
        let second = render("{{ x | codename }}", &vars_x());
        match (first, second) {
            (Ok(a), Ok(b)) => {
                assert_eq!(a, b, "codename is deterministic");
                assert!(a.contains('-'), "n-1 adjectives + 1 noun: {a:?}");
                assert!(!a.contains(' '), "hyphen-joined: {a:?}");
            }
            (Err(e), _) | (_, Err(e)) => panic!("codename failed: {e:?}"),
        }
    }

    #[test]
    fn codename_honors_the_word_count() {
        let rendered = render("{{ x | codename(3) }}", &vars_x()).unwrap();
        assert_eq!(rendered.matches('-').count(), 2, "3 words: {rendered:?}");
    }

    fn vars_x() -> TemplateVars {
        let mut vars = TemplateVars::new();
        vars.insert("x", "seed".to_string());
        vars
    }
}
