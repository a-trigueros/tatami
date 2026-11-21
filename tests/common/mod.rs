//! Test harness: a throwaway real jj repo. Proof tier — requires the `jj`
//! binary on PATH; runs everywhere jj exists (CI installs it).

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// A fresh jj repo in a temp directory, cleaned up on drop.
pub struct TempRepo {
    dir: TempDir,
}

impl TempRepo {
    /// `jj git init repo` with an isolated author config (`JJ_CONFIG`), so
    /// tests never touch the developer's real jj config.
    pub fn init() -> Self {
        let dir = TempDir::new().expect("tempdir for test repo");
        let config_path = dir.path().join("jj-config.toml");
        std::fs::write(
            &config_path,
            "[user]\nname = \"Tatami Tests\"\nemail = \"tatami-tests@example.com\"\n",
        )
        .expect("write jj test config");

        let repo_path = dir.path().join("repo");
        let output = Command::new("jj")
            .arg("git")
            .arg("init")
            .arg(&repo_path)
            .env("JJ_CONFIG", &config_path)
            .output()
            .expect("jj binary must be installed and on PATH");
        assert!(
            output.status.success(),
            "jj git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        Self { dir }
    }

    /// Root of the freshly created repo. Canonicalized because jj resolves
    /// symlinks in the paths it prints (macOS `/var` → `/private/var`).
    pub fn path(&self) -> PathBuf {
        self.dir.path().join("repo").canonicalize().unwrap()
    }

    /// Directory hosting the repo — itself not a repo (for "outside a repo"
    /// cases).
    pub fn outside_dir(&self) -> &Path {
        self.dir.path()
    }
}
