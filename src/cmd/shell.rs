//! `tt config shell init` — prints the shell integration; tatami never
//! writes rc files. The block defines a `tt()` wrapper that intercepts
//! `switch` and `merge` (both print the destination path on stdout and the
//! wrapper `cd`s to it) and sources the completions dynamically. The user
//! pastes the block into their rc — or evals it:
//! `eval "$(tatami config shell init zsh)"`.

use std::path::Path;

use clap::CommandFactory;
use clap_complete::generate;
use thiserror::Error;

use crate::cli::Cli;

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
}

/// Environment-derived inputs, captured once in `main` (the IO boundary).
#[derive(Debug, Clone)]
pub struct ShellEnv {
    /// Raw `$SHELL`; `None` when unset.
    pub env_shell: Option<String>,
}

impl ShellEnv {
    fn resolve_shell(&self, override_shell: Option<&str>) -> Result<Shell, ShellError> {
        let requested = override_shell
            .or(self.env_shell.as_deref())
            .ok_or(ShellError::NoShellDetected)?;
        Shell::parse(requested).ok_or_else(|| ShellError::UnsupportedShell {
            requested: requested.to_string(),
        })
    }
}

/// The integration block for the resolved shell — ready to paste into the rc.
pub fn init(env: &ShellEnv, override_shell: Option<&str>) -> Result<String, ShellError> {
    let shell = env.resolve_shell(override_shell)?;
    Ok(integration_block(shell))
}

/// Prints the completion script for `shell` — sourced by the init block, or
/// saved to a fpath directory for a static setup.
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
    #[error(
        "cannot detect the shell — pass one: `tatami config shell init zsh|bash` (or set $SHELL)"
    )]
    NoShellDetected,
}

const START_MARKER: &str = "# >>> tatami shell integration >>>";
const END_MARKER: &str = "# <<< tatami shell integration <<<";

/// The rc block for `shell`, markers included so users can find and replace
/// it later. The wrapper intercepts `switch` and `merge`: both subcommands
/// print their destination (landing) path on stdout and the wrapper cds to
/// it; everything else — and any empty stdout — passes through untouched.
fn integration_block(shell: Shell) -> String {
    let header = format!(
        "{START_MARKER}\n\
         # Added by tatami — paste into your rc, or: eval \"$(tatami config shell init {})\"\n",
        shell.label(),
    );
    let footer = format!("\n{END_MARKER}");
    match shell {
        Shell::Zsh => format!(
            "{header}\n\
             # zsh: place after `compinit` so completions register.\n\
             tt() {{\n  \
             if [[ \"$1\" == \"switch\" || \"$1\" == \"merge\" ]]; then\n    \
             local destination\n    \
             if destination=$(command tatami \"$@\") && [[ -n \"$destination\" ]]; then\n      \
             cd \"$destination\"\n    \
             fi\n    \
             return $?\n  \
             fi\n  \
             command tatami \"$@\"\n\
             }}\n\
             source <(tatami config shell completion zsh){footer}"
        ),
        Shell::Bash => format!(
            "{header}\n\
             tt() {{\n  \
             if [ \"$1\" = \"switch\" ] || [ \"$1\" = \"merge\" ]; then\n    \
             local destination\n    \
             if destination=$(command tatami \"$@\") && [ -n \"$destination\" ]; then\n      \
             cd \"$destination\"\n    \
             fi\n    \
             return $?\n  \
             fi\n  \
             command tatami \"$@\"\n\
             }}\n\
             source <(tatami config shell completion bash){footer}"
        ),
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

    #[test]
    fn parses_shell_names_and_interpreter_paths() {
        assert_eq!(Shell::parse("zsh"), Some(Shell::Zsh));
        assert_eq!(Shell::parse("/bin/bash"), Some(Shell::Bash));
        assert_eq!(Shell::parse("fish"), None);
    }

    #[test]
    fn generates_a_block_with_markers_and_the_wrapper() {
        let block = integration_block(Shell::Zsh);

        assert!(block.starts_with(START_MARKER));
        assert!(block.ends_with(END_MARKER));
        assert!(block.contains("command tatami \"$@\""));
        assert!(block.contains("cd \"$destination\""));
    }

    #[test]
    fn the_wrapper_intercepts_switch_and_merge() {
        for shell in [Shell::Zsh, Shell::Bash] {
            let block = integration_block(shell);
            let interceptor = if shell == Shell::Zsh {
                "[[ \"$1\" == \"switch\" || \"$1\" == \"merge\" ]]"
            } else {
                "[ \"$1\" = \"switch\" ] || [ \"$1\" = \"merge\" ]"
            };
            assert!(
                block.contains(interceptor),
                "{:?} block must intercept switch and merge: {block:?}",
                shell.label()
            );
        }
    }

    #[test]
    fn wires_completions_with_the_full_config_shell_completion_path() {
        // Regression: the block once emitted `tatami config completion …`,
        // which clap rejects (`unrecognized subcommand`) at every shell start.
        for shell in [Shell::Zsh, Shell::Bash] {
            let block = integration_block(shell);
            assert!(
                block.contains(&format!(
                    "source <(tatami config shell completion {})",
                    shell.label()
                )),
                "{:?} block must source its completions: {block:?}",
                shell.label()
            );
            assert!(
                !block.contains("tatami config completion"),
                "{:?} block must not use the wrong path: {block:?}",
                shell.label()
            );
        }
    }

    #[test]
    fn resolves_the_shell_from_the_override_then_from_env() {
        let env = ShellEnv {
            env_shell: Some("/usr/bin/fish".to_string()),
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
        let env = ShellEnv { env_shell: None };

        assert!(matches!(
            env.resolve_shell(None),
            Err(ShellError::NoShellDetected)
        ));
    }

    #[test]
    fn the_commands_the_output_references_are_recognized_by_the_cli() {
        // The block's header and source line tell the user to run concrete
        // commands; every one of them must parse, or shell startup breaks.
        for shell in [Shell::Zsh, Shell::Bash] {
            let block = integration_block(shell);
            let label = shell.label();

            // `eval "$(tatami config shell init zsh)"` from the header.
            Cli::command()
                .try_get_matches_from(["tatami", "config", "shell", "init", label])
                .unwrap_or_else(|error| {
                    panic!("init must accept a positional shell: {error}");
                });

            // The exact `source <(...)` line emitted by the block.
            let source_line = block
                .lines()
                .find(|line| line.starts_with("source <("))
                .unwrap_or_else(|| panic!("no source line in the {label} block"));
            let command = source_line
                .trim_start_matches("source <(")
                .trim_end_matches(')');
            let argv: Vec<&str> = std::iter::once("tatami")
                .chain(command.split_whitespace())
                .collect();
            Cli::command()
                .try_get_matches_from(&argv)
                .unwrap_or_else(|error| {
                    panic!("the block references `{command}`, which the CLI rejects: {error}");
                });
        }
    }

    #[test]
    fn generates_completion_scripts_for_both_shells() {
        let zsh = completion_script(Shell::Zsh);
        let bash = completion_script(Shell::Bash);

        assert!(zsh.contains("#compdef tatami"), "zsh: {zsh:?}");
        assert!(bash.contains("_tatami()"), "bash: {bash:?}");
    }
}
