//! `tt config shell …` — install/remove the shell integration.
//!
//! The integration is a marked block appended to the rc file: a `tt()`
//! function that intercepts `switch` and `merge --remove` (both print the
//! destination path on stdout and the wrapper `cd`s to it), passing
//! everything else through. Completions are written as files (zsh: a fpath
//! directory wired by the block; bash: the standard bash-completion
//! directory), never `eval`-ed at startup.

use std::fs;
use std::path::{Path, PathBuf};

use clap::CommandFactory;
use clap_complete::generate;
use thiserror::Error;

use crate::cli::Cli;
use crate::paths::ConfigEnv;

/// The shells tatami integrates with today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Zsh,
    Bash,
}

impl Shell {
    fn parse(value: &str) -> Option<Self> {
        // Accept both `zsh` and a full interpreter path like `/bin/zsh`.
        let name = Path::new(value)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(value);
        match name {
            "zsh" => Some(Self::Zsh),
            "bash" => Some(Self::Bash),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
        }
    }

    fn rc_file_name(self) -> &'static str {
        match self {
            Self::Zsh => ".zshrc",
            Self::Bash => ".bashrc",
        }
    }
}

/// Environment-derived inputs, captured once in `main` (the IO boundary).
#[derive(Debug, Clone)]
pub struct ShellEnv {
    /// Raw `$SHELL`; `None` when unset.
    pub env_shell: Option<String>,
    pub home: PathBuf,
    /// tatami's config dir (`…/tatami`), hosting generated completions.
    pub config_dir: PathBuf,
}

impl ShellEnv {
    pub fn from_config_env(config: &ConfigEnv, env_shell: Option<String>) -> Self {
        Self {
            env_shell,
            home: config.home().to_path_buf(),
            config_dir: config.tatami_config_dir(),
        }
    }

    fn resolve_shell(&self, override_shell: Option<&str>) -> Result<Shell, ShellError> {
        let requested = override_shell
            .or(self.env_shell.as_deref())
            .ok_or(ShellError::NoShellDetected)?;
        Shell::parse(requested).ok_or_else(|| ShellError::UnsupportedShell {
            requested: requested.to_string(),
        })
    }
}

pub fn install(env: &ShellEnv, override_shell: Option<&str>) -> Result<(), ShellError> {
    let shell = env.resolve_shell(override_shell)?;
    let rc_path = env.home.join(shell.rc_file_name());
    let block = integration_block(shell);
    write_block(&rc_path, &block)?;
    let completion_path = write_completion(env, shell)?;
    println!("installed: {rc_path:?} ({})", shell.label());
    println!("completions: {completion_path:?}");
    println!("restart your shell (or `source` the file) to get the `tt` wrapper");
    Ok(())
}

pub fn uninstall(env: &ShellEnv, override_shell: Option<&str>) -> Result<(), ShellError> {
    let shell = env.resolve_shell(override_shell)?;
    let rc_path = env.home.join(shell.rc_file_name());
    let removed = remove_block_from(&rc_path)?;
    remove_completion(env, shell)?;
    if removed {
        println!("removed integration from {rc_path:?}");
    } else {
        println!("no integration found in {rc_path:?}");
    }
    Ok(())
}

/// Prints the completion script for `shell` — for manual wiring (fish, …)
/// without touching rc files.
pub fn completion(shell: &str) -> Result<String, ShellError> {
    let shell = Shell::parse(shell).ok_or_else(|| ShellError::UnsupportedShell {
        requested: shell.to_string(),
    })?;
    Ok(completion_script(shell))
}

#[derive(Debug, Error)]
pub enum ShellError {
    #[error("shell `{requested}` is not supported (try zsh or bash)")]
    UnsupportedShell { requested: String },
    #[error("cannot detect the shell — pass `--shell zsh|bash` (or set $SHELL)")]
    NoShellDetected,
    #[error("cannot update {path}: {source}")]
    RcWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot read {path}: {source}")]
    RcRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot write completions to {path}: {source}")]
    CompletionWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

const START_MARKER: &str = "# >>> tatami shell integration >>>";
const END_MARKER: &str = "# <<< tatami shell integration <<<";

/// The rc block for `shell`, markers included. `remove_block` strips exactly
/// what this produces, so installs are idempotent and upgradable.
fn integration_block(shell: Shell) -> String {
    let header = format!(
        "{START_MARKER}\n# Added by `tatami config shell install`. Remove with `tatami config shell uninstall`.\n"
    );
    let footer = format!("\n{END_MARKER}");
    match shell {
        Shell::Zsh => format!(
            "{header}\
fpath=(\"${{XDG_CONFIG_HOME:-$HOME/.config}}/tatami/completions\" $fpath)\n\
autoload -Uz compinit\n\
compinit\n\
tt() {{\n  \
if [[ \"$1\" == \"switch\" ]]; then\n    \
shift\n    \
local destination\n    \
if destination=$(command tatami switch \"$@\") && [[ -n \"$destination\" ]]; then\n      \
cd \"$destination\"\n    \
fi\n    \
return $?\n  \
fi\n  \
command tatami \"$@\"\n\
}}{footer}"
        ),
        Shell::Bash => format!(
            "{header}\
tt() {{\n  \
if [ \"$1\" = \"switch\" ]; then\n    \
shift\n    \
local destination\n    \
if destination=$(command tatami switch \"$@\") && [ -n \"$destination\" ]; then\n      \
cd \"$destination\"\n    \
fi\n    \
return $?\n  \
fi\n  \
command tatami \"$@\"\n\
}}{footer}"
        ),
    }
}

/// Rewrites `rc_path` with exactly one integration block (the newest),
/// preserving everything around it. Creates the file when missing.
fn write_block(rc_path: &Path, block: &str) -> Result<(), ShellError> {
    let existing = match fs::read_to_string(rc_path) {
        Ok(content) => content,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(ShellError::RcRead {
                path: rc_path.to_path_buf(),
                source,
            });
        }
    };
    let mut content = remove_block(&existing);
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(block);
    content.push('\n');
    fs::write(rc_path, content).map_err(|source| ShellError::RcWrite {
        path: rc_path.to_path_buf(),
        source,
    })
}

/// Returns `true` when a block was present and removed.
fn remove_block_from(rc_path: &Path) -> Result<bool, ShellError> {
    let content = match fs::read_to_string(rc_path) {
        Ok(content) => content,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(ShellError::RcRead {
                path: rc_path.to_path_buf(),
                source,
            });
        }
    };
    let stripped = remove_block(&content);
    if stripped == content {
        return Ok(false);
    }
    fs::write(rc_path, stripped).map_err(|source| ShellError::RcWrite {
        path: rc_path.to_path_buf(),
        source,
    })?;
    Ok(true)
}

/// Removes every marked block; content outside the markers is untouched.
fn remove_block(content: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut in_block = false;
    for line in content.lines() {
        if line.trim() == START_MARKER {
            in_block = true;
            continue;
        }
        if in_block {
            if line.trim() == END_MARKER {
                in_block = false;
            }
            continue;
        }
        kept.push(line);
    }
    let mut result = kept.join("\n");
    if content.ends_with('\n') && !result.is_empty() && !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

fn completion_file_path(env: &ShellEnv, shell: Shell) -> PathBuf {
    match shell {
        // A fpath directory wired by the zsh block.
        Shell::Zsh => env.config_dir.join("completions").join("_tatami"),
        // The directory bash-completion scans automatically.
        Shell::Bash => env
            .home
            .join(".local/share/bash-completion/completions/tatami"),
    }
}

fn write_completion(env: &ShellEnv, shell: Shell) -> Result<PathBuf, ShellError> {
    let path = completion_file_path(env, shell);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ShellError::CompletionWrite {
            path: path.clone(),
            source,
        })?;
    }
    fs::write(&path, completion_script(shell)).map_err(|source| ShellError::CompletionWrite {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

fn remove_completion(env: &ShellEnv, shell: Shell) -> Result<(), ShellError> {
    let path = completion_file_path(env, shell);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ShellError::CompletionWrite { path, source }),
    }
}

/// clap_complete output is UTF-8 by construction.
fn completion_script(shell: Shell) -> String {
    let mut command = Cli::command();
    let mut buf: Vec<u8> = Vec::new();
    match shell {
        Shell::Zsh => generate(clap_complete::shells::Zsh, &mut command, "tatami", &mut buf),
        Shell::Bash => generate(
            clap_complete::shells::Bash,
            &mut command,
            "tatami",
            &mut buf,
        ),
    }
    String::from_utf8(buf).expect("clap_complete emits UTF-8")
}

#[cfg(test)]
mod shell_should {
    use super::*;
    use pretty_assertions::assert_str_eq;

    #[test]
    fn parses_shell_names_and_interpreter_paths() {
        assert_eq!(Shell::parse("zsh"), Some(Shell::Zsh));
        assert_eq!(Shell::parse("/bin/bash"), Some(Shell::Bash));
        assert_eq!(Shell::parse("fish"), None);
    }

    #[test]
    fn generates_a_block_with_markers_and_the_switch_wrapper() {
        let block = integration_block(Shell::Zsh);

        assert!(block.starts_with(START_MARKER));
        assert!(block.ends_with(END_MARKER));
        assert!(block.contains("command tatami switch \"$@\""));
        assert!(block.contains("cd \"$destination\""));
        assert!(block.contains("command tatami \"$@\""));
    }

    #[test]
    fn wires_zsh_completions_through_fpath_but_not_bash() {
        let zsh = integration_block(Shell::Zsh);
        let bash = integration_block(Shell::Bash);

        assert!(zsh.contains("tatami/completions"));
        assert!(zsh.contains("compinit"));
        assert!(!bash.contains("fpath"));
        assert!(!bash.contains("compinit"));
    }

    #[test]
    fn installs_are_idempotent_and_replace_the_previous_block() {
        let rc = tempfile::tempdir().unwrap();
        let path = rc.path().join(".zshrc");
        fs::write(&path, "export EDITOR=helix\n").unwrap();
        let old_block = integration_block(Shell::Zsh);
        fs::write(&path, format!("export EDITOR=helix\n{old_block}\n")).unwrap();

        write_block(&path, &integration_block(Shell::Bash)).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(
            content.matches(START_MARKER).count(),
            1,
            "content: {content:?}"
        );
        assert!(content.contains("export EDITOR=helix"));
        assert!(content.contains("if [ \"$1\" = \"switch\" ]"));
    }

    #[test]
    fn uninstall_removes_only_the_block() {
        let rc = tempfile::tempdir().unwrap();
        let path = rc.path().join(".bashrc");
        let block = integration_block(Shell::Bash);
        fs::write(&path, format!("export EDITOR=helix\n{block}\necho done\n")).unwrap();

        let removed = remove_block_from(&path).unwrap();

        assert!(removed);
        let content = fs::read_to_string(&path).unwrap();
        assert_str_eq!(content, "export EDITOR=helix\necho done\n");
    }

    #[test]
    fn uninstall_of_a_file_without_a_block_changes_nothing() {
        let rc = tempfile::tempdir().unwrap();
        let path = rc.path().join(".zshrc");
        fs::write(&path, "export EDITOR=helix\n").unwrap();

        let removed = remove_block_from(&path).unwrap();

        assert!(!removed);
        assert_str_eq!(fs::read_to_string(&path).unwrap(), "export EDITOR=helix\n");
    }

    #[test]
    fn resolves_the_shell_from_the_override_then_from_env() {
        let env = ShellEnv {
            env_shell: Some("/usr/bin/fish".to_string()),
            home: PathBuf::from("/home/u"),
            config_dir: PathBuf::from("/home/u/.config/tatami"),
        };

        assert_eq!(
            env.resolve_shell(Some("bash")).unwrap(),
            Shell::Bash,
            "the explicit override wins over an unsupported $SHELL"
        );
        assert!(matches!(
            env.resolve_shell(None),
            Err(ShellError::UnsupportedShell { .. })
        ));
    }

    #[test]
    fn fails_to_resolve_when_no_shell_source_exists() {
        let env = ShellEnv {
            env_shell: None,
            home: PathBuf::from("/home/u"),
            config_dir: PathBuf::from("/home/u/.config/tatami"),
        };

        assert!(matches!(
            env.resolve_shell(None),
            Err(ShellError::NoShellDetected)
        ));
    }

    #[test]
    fn generates_completion_scripts_for_both_shells() {
        let zsh = completion_script(Shell::Zsh);
        let bash = completion_script(Shell::Bash);

        assert!(zsh.contains("#compdef tatami"), "zsh: {zsh:?}");
        assert!(bash.contains("_tatami()"), "bash: {bash:?}");
    }
}
