//! Proof tier: the generated shell integration, installed in a fake HOME,
//! must actually `cd` — verified by real zsh and bash processes sourcing the
//! real rc file and running the real binary.
//!
//! `zsh -i -c` does not source `.zshrc` on macOS (verified), so the test
//! sources the rc explicitly; what is under test is the block's content, not
//! the shell's startup rules.

mod common;

use common::TempRepo;
use std::path::{Path, PathBuf};
use std::process::Command;
use tatami::cmd::shell::{self, ShellEnv};

fn fake_home() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn fake_env(home: &Path) -> ShellEnv {
    ShellEnv {
        env_shell: None,
        home: home.to_path_buf(),
        config_dir: home.join(".config/tatami"),
    }
}

/// Runs `interpreter -c '<script>'` with the fake HOME and the tatami binary
/// on PATH. `None` = interpreter not installed here (skip).
fn run_in_shell(
    interpreter: &str,
    script: &str,
    home: &Path,
    repo: &TempRepo,
) -> Option<std::process::Output> {
    let output = Command::new(interpreter)
        .arg("-c")
        .arg(script)
        .current_dir(repo.path())
        .env("HOME", home)
        .env("JJ_CONFIG", repo.jj_config_path())
        .env(
            "PATH",
            format!(
                "{}:{}",
                PathBuf::from(env!("CARGO_BIN_EXE_tatami"))
                    .parent()
                    .unwrap()
                    .display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .output();
    match output {
        Ok(output) => Some(output),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => panic!("cannot spawn {interpreter}: {source}"),
    }
}

#[test]
fn the_installed_wrapper_cds_to_the_switched_workspace() {
    let repo = TempRepo::init();
    repo.describe_working_copy("main workspace");
    repo.add_workspace("ws2");
    let ws2_root = repo.path().parent().unwrap().join("ws2");

    let home = fake_home();
    let home_path = home.path().to_path_buf();
    shell::install(&fake_env(&home_path), Some("zsh")).expect("zsh install");
    shell::install(&fake_env(&home_path), Some("bash")).expect("bash install");

    for interpreter in ["zsh", "bash"] {
        let rc = home.path().join(if interpreter == "zsh" {
            ".zshrc"
        } else {
            ".bashrc"
        });
        let output = match run_in_shell(
            interpreter,
            &format!("source {rc:?}; tt switch ws2 && pwd"),
            &home_path,
            &repo,
        ) {
            Some(output) => output,
            None => {
                println!("{interpreter} is not installed; skipping this proof");
                continue;
            }
        };

        assert!(
            output.status.success(),
            "{interpreter} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let last_line = stdout.lines().last().expect("pwd output");
        assert_eq!(
            Path::new(last_line),
            ws2_root.as_path(),
            "{interpreter} did not cd: {stdout:?}"
        );
    }
}

/// `tt <other>` passes through to the binary (no cd side effects).
#[test]
fn the_wrapper_passes_other_commands_through() {
    let repo = TempRepo::init();

    let home = fake_home();
    shell::install(&fake_env(home.path()), Some("bash")).expect("bash install");

    let output = match run_in_shell(
        "bash",
        &format!("source {:?}; tt list", home.path().join(".bashrc")),
        home.path(),
        &repo,
    ) {
        Some(output) => output,
        None => {
            println!("bash is not installed; skipping this proof");
            return;
        }
    };

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("default"),
        "tt list output missing workspaces: {stdout:?}"
    );
}
