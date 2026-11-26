//! `tt config shell init` — prints the shell integration; tatami never
//! writes rc files. The block defines a `tt()` wrapper that intercepts
//! `switch`/`sw` and `merge`/`mg` (both print the destination path on stdout
//! and the wrapper `cd`s to it) and sources the completions dynamically. The user
//! pastes the block into their rc — or evals it:
//! `eval "$(tatami config shell init zsh)"`.
//!
//! The completion scripts are hand-written (not clap_complete): they must
//! offer *dynamic* candidates — workspaces for switch/remove, bookmarks for
//! merge/rebase — by calling the hidden `tatami __complete <target>`.

use std::path::Path;

use thiserror::Error;

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
/// it later. The wrapper intercepts `switch`/`sw` and `merge`/`mg`: both
/// subcommands print their destination (landing) path on stdout and the wrapper cds to
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
             if [[ \"$1\" == \"switch\" || \"$1\" == \"sw\" || \"$1\" == \"merge\" || \"$1\" == \"mg\" ]]; then\n    \
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
             if [ \"$1\" = \"switch\" ] || [ \"$1\" = \"sw\" ] || [ \"$1\" = \"merge\" ] || [ \"$1\" = \"mg\" ]; then\n    \
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

/// The completion script for `shell` — hand-written so candidates can be
/// dynamic: `switch`/`remove` complete workspace names and `merge`/`rebase`
/// complete bookmark names via the hidden `tatami __complete <target>`.
/// Registered for both the binary and the `tt` wrapper.
fn completion_script(shell: Shell) -> String {
    match shell {
        Shell::Zsh => r#"#compdef tatami tt
# tatami completions — dynamic (workspaces, bookmarks). Regenerate with
# `tatami config shell init`; place this after `compinit` in your rc.
_tatami() {
  if (( CURRENT == 2 )); then
    local -a cmds=(list ls switch sw merge mg rebase rb remove rm step config)
    _describe -t tatami-cmds command cmds
    return
  fi
  local sub="${words[2]}" candidates
  case "$sub" in
    switch|sw|remove|rm|merge|mg|rebase|rb)
      candidates=("${(f)$(command tatami __complete "$sub" 2>/dev/null)}")
      (( ${#candidates} )) && _describe -t tatami-candidates "$sub" candidates
      ;;
  esac
}
(( $+functions[compdef] )) && compdef _tatami tatami tt
"#
        .to_string(),
        Shell::Bash => r#"# tatami completions — dynamic (workspaces, bookmarks). Regenerate with
# `tatami config shell init`.
_tatami() {
  local cur="${COMP_WORDS[COMP_CWORD]}"
  if [ "$COMP_CWORD" -eq 1 ]; then
    COMPREPLY=($(compgen -W "list ls switch sw merge mg rebase rb remove rm step config" -- "$cur"))
    return
  fi
  case "${COMP_WORDS[1]}" in
    switch|sw|remove|rm|merge|mg|rebase|rb)
      COMPREPLY=($(compgen -W "$(command tatami __complete "${COMP_WORDS[1]}" 2>/dev/null)" -- "$cur"))
      ;;
  esac
}
complete -F _tatami tatami tt 2>/dev/null
"#
        .to_string(),
    }
}

#[cfg(test)]
mod shell_should {
    use super::*;
    use crate::cli::Cli;
    use clap::CommandFactory as _;

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
    fn the_wrapper_intercepts_switch_and_merge_and_their_shortcuts() {
        for shell in [Shell::Zsh, Shell::Bash] {
            let block = integration_block(shell);
            let interceptor = if shell == Shell::Zsh {
                "[[ \"$1\" == \"switch\" || \"$1\" == \"sw\" || \"$1\" == \"merge\" || \"$1\" == \"mg\" ]]"
            } else {
                "[ \"$1\" = \"switch\" ] || [ \"$1\" = \"sw\" ] || [ \"$1\" = \"merge\" ] || [ \"$1\" = \"mg\" ]"
            };
            assert!(
                block.contains(interceptor),
                "{:?} block must intercept switch/merge and their shortcuts: {block:?}",
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

            // The candidates command the completion script calls.
            Cli::command()
                .try_get_matches_from(["tatami", "__complete", label])
                .unwrap_or_else(|error| {
                    panic!("__complete must accept `{label}`: {error}");
                });
        }
    }

    #[test]
    fn completion_scripts_wire_dynamic_candidates_through_the_hidden_command() {
        for shell in [Shell::Zsh, Shell::Bash] {
            let script = completion_script(shell);
            assert!(
                script.contains("tatami __complete"),
                "{:?} script must call the hidden candidates command: {script:?}",
                shell.label()
            );
            assert!(
                script.contains("switch|sw|remove|rm|merge|mg|rebase|rb"),
                "{:?} script must complete workspaces and bookmarks for shortcuts too: {script:?}",
                shell.label()
            );
        }
    }

    #[test]
    fn completion_scripts_advertise_the_shortcuts() {
        let commands = "list ls switch sw merge mg rebase rb remove rm step config";
        for shell in [Shell::Zsh, Shell::Bash] {
            let script = completion_script(shell);
            assert!(
                script.contains(commands),
                "{:?} script must list every command and shortcut: {script:?}",
                shell.label()
            );
        }
    }

    #[test]
    fn completions_register_for_both_the_binary_and_the_tt_wrapper() {
        let zsh = completion_script(Shell::Zsh);
        let bash = completion_script(Shell::Bash);

        assert!(zsh.contains("#compdef tatami tt"), "zsh: {zsh:?}");
        assert!(
            zsh.contains("compdef _tatami tatami tt"),
            "the tt wrapper must complete too: {zsh:?}"
        );
        assert!(bash.contains("_tatami()"), "bash: {bash:?}");
        assert!(
            bash.contains("complete -F _tatami tatami tt"),
            "the tt wrapper must complete too: {bash:?}"
        );
    }
}
