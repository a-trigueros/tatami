//! Adapter to the `jj` CLI — the only place tatami invokes jujutsu.
//!
//! jj has no stable Rust library: the subprocess interface with template
//! output (`-T`) is the only durable contract. Everything else in tatami
//! depends on the [`Jj`] trait, never on jj internals.

use std::path::PathBuf;
use std::process::Command;
use thiserror::Error;

/// Operations tatami needs from jujutsu. Faked in fast tests, proven against
/// the real binary in integration tests.
pub trait Jj {
    /// Absolute root directory of the current workspace.
    fn workspace_root(&self) -> Result<PathBuf, JjError>;
}

/// Invokes the real `jj` binary.
#[derive(Debug, Clone)]
pub struct CliJj {
    working_dir: PathBuf,
}

impl CliJj {
    /// Adapter rooted at `working_dir`: every jj invocation runs from there.
    pub fn at(working_dir: PathBuf) -> Self {
        Self { working_dir }
    }

    /// Runs `jj <args>` from the adapter's working directory and returns the
    /// trimmed stdout on success.
    fn run(&self, args: &[&str]) -> Result<String, JjError> {
        let output = Command::new("jj")
            .args(args)
            .current_dir(&self.working_dir)
            .output()
            .map_err(|source| JjError::Spawn { source })?;
        if !output.status.success() {
            return Err(JjError::CommandFailed {
                command: format!("jj {}", args.join(" ")),
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

impl Jj for CliJj {
    fn workspace_root(&self) -> Result<PathBuf, JjError> {
        let root = self.run(&["workspace", "root"])?;
        Ok(PathBuf::from(root))
    }
}

#[derive(Debug, Error)]
pub enum JjError {
    /// `jj` could not be launched at all (missing binary, permissions).
    #[error("cannot run jj (is it installed and on PATH?): {source}")]
    Spawn {
        #[source]
        source: std::io::Error,
    },
    /// jj ran and rejected the operation; stderr is carried so callers can
    /// surface jj's own diagnosis.
    #[error("`{command}` failed with {status}\n{stderr}")]
    CommandFailed {
        command: String,
        status: std::process::ExitStatus,
        stderr: String,
    },
}
