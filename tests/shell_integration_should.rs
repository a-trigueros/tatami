//! Proof tier: the printed shell integration, pasted into a fake rc, must
//! actually `cd` — verified by real zsh and bash processes sourcing the rc
//! and running the real binary.
//!
//! `zsh -i -c` does not source `.zshrc` on macOS (verified), so the test
//! sources the rc explicitly; what is under test is the block's content, not
//! the shell's startup rules.

mod common;

use common::TempRepo;
use std::path::{Path, PathBuf};
use std::process::Command;
use tatami::cmd::shell::{self, ShellEnv};

fn fake_env() -> ShellEnv {
    ShellEnv { env_shell: None }
}

/// The integration block for `shell`, as the user would paste it into their
/// rc — tatami itself never writes rc files.
fn paste_block(home: &Path, shell: &str) {
    let block = shell::init(&fake_env(), Some(shell)).expect("init block");
    let rc = home.join(match shell {
        "zsh" => ".zshrc",
        _ => ".bashrc",
    });
    std::fs::write(rc, block).unwrap();
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
fn the_pasted_wrapper_cds_to_the_switched_workspace() {
    let repo = TempRepo::init();
    repo.describe_working_copy("main workspace");
    repo.add_workspace("ws2");
    let ws2_root = repo.path().parent().unwrap().join("ws2");

    let home = tempfile::tempdir().unwrap();
    let home_path = home.path().to_path_buf();
    paste_block(&home_path, "zsh");
    paste_block(&home_path, "bash");

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
        // The block sources completions dynamically; a wrong command path
        // would print clap's error here without failing the cd (regression
        // guard: the completion wiring must not error at shell startup).
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("unrecognized subcommand"),
            "{interpreter} startup errors: {stderr:?}"
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

/// `tt merge --remove` integrates, forgets the workspace, and the wrapper
/// brings the caller back to the landing root (the host workspace here).
#[test]
fn the_pasted_wrapper_cds_back_after_merge_remove() {
    let repo = TempRepo::init();
    repo.capture_jj(&["bookmark", "create", "main", "-r", "@"]);
    repo.describe_working_copy("main workspace");
    repo.add_workspace("ws1");
    let ws1_root = repo.path().parent().unwrap().join("ws1");
    std::fs::write(ws1_root.join("ws1.txt"), "content").unwrap();
    repo.capture_jj_in(&ws1_root, &["commit", "-m", "feature"]);
    repo.capture_jj_in(&ws1_root, &["bookmark", "create", "ws1", "-r", "ws1@-"]);

    let home = tempfile::tempdir().unwrap();
    let home_path = home.path().to_path_buf();
    paste_block(&home_path, "bash");

    let output = match run_in_shell(
        "bash",
        &format!(
            "source {:?}; tt merge --remove && pwd",
            home.path().join(".bashrc")
        ),
        &home_path,
        &repo,
    ) {
        Some(output) => output,
        None => {
            println!("bash is not installed; skipping this proof");
            return;
        }
    };

    assert!(
        output.status.success(),
        "merge --remove failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let last_line = stdout.lines().last().expect("pwd output");
    assert_eq!(
        Path::new(last_line),
        repo.path(),
        "must land back on the host root: {stdout:?}"
    );
}

/// The switch contract the wrapper relies on: with `--create`, tatami prints
/// the freshly created workspace's path — and nothing else — on stdout.
#[test]
fn switch_create_prints_the_new_workspace_path_on_stdout() {
    let repo = TempRepo::init();
    repo.describe_working_copy("main workspace");
    // Config isolation: tests must never read the machine's real tatami or
    // worktrunk config (its template points at ~/factory).
    let home = tempfile::tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_tatami"))
        .args(["switch", "--create", "ws3"])
        .current_dir(repo.path())
        .env("JJ_CONFIG", repo.jj_config_path())
        .env("HOME", home.path())
        .output()
        .expect("run tatami switch --create");

    assert!(
        output.status.success(),
        "switch --create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Default layout: a sibling of the repo dir named `<repo>.<workspace>`.
    let repo_name = repo
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let expected = repo
        .path()
        .parent()
        .unwrap()
        .join(format!("{repo_name}.ws3"));
    assert_eq!(
        Path::new(stdout.trim()),
        expected,
        "stdout must be exactly the destination (the wrapper cds to it): {stdout:?}"
    );
    assert!(expected.exists(), "the workspace must have been created");
}

/// `tt <other>` passes through to the binary (no cd side effects).
#[test]
fn the_wrapper_passes_other_commands_through() {
    let repo = TempRepo::init();

    let home = tempfile::tempdir().unwrap();
    paste_block(home.path(), "bash");

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
