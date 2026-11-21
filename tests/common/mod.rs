//! Test harness: a throwaway real jj repo. Proof tier — requires the `jj`
//! binary on PATH; runs everywhere jj exists (CI installs it).

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// A fresh jj repo in a temp directory, cleaned up on drop.
pub struct TempRepo {
    dir: TempDir,
    config_path: PathBuf,
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
            // The repo dir does not exist yet; run from the temp dir itself.
            .current_dir(dir.path())
            .env("JJ_CONFIG", &config_path)
            .output()
            .expect("jj binary must be installed and on PATH");
        assert!(
            output.status.success(),
            "jj git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        Self { dir, config_path }
    }

    fn run_jj(&self, args: &[&str]) {
        let output = Command::new("jj")
            .args(args)
            .current_dir(self.dir.path().join("repo"))
            .env("JJ_CONFIG", &self.config_path)
            .output()
            .expect("jj binary must be installed and on PATH");
        assert!(
            output.status.success(),
            "jj {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
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

    /// Describes the current workspace's working-copy commit.
    pub fn describe_working_copy(&self, message: &str) {
        self.run_jj(&["describe", "-m", message]);
    }

    /// Adds a secondary workspace as a sibling directory of the repo root.
    pub fn add_workspace(&self, name: &str) {
        let path = self.outside_dir().join(name);
        let path_str = path.to_str().expect("utf-8 temp path");
        self.run_jj(&["workspace", "add", path_str]);
    }
}
