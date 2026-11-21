//! Filesystem locations tatami reads and writes.
//!
//! Path resolution is pure: [`ConfigEnv`] captures environment-derived inputs
//! once (the IO boundary) and every function taking it can be tested without
//! touching the process environment.

use std::env;
use std::path::PathBuf;

use serde::Serialize;
use thiserror::Error;

/// Which config file a setting came from. The user config resolves to
/// tatami's own file first and falls back to worktrunk's, keeping the two
/// tools interchangeable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UserConfigSource {
    /// tatami's own user config (or `TATAMI_CONFIG_PATH`).
    Tatami,
    /// worktrunk's user config, used only when no tatami config exists.
    WorktrunkFallback,
}

impl UserConfigSource {
    /// Human-readable provenance for `tt config show`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Tatami => "tatami",
            Self::WorktrunkFallback => "worktrunk fallback",
        }
    }
}

/// Home directory could not be determined; path resolution cannot proceed.
#[derive(Debug, Error)]
#[error("cannot determine the home directory")]
pub struct HomeDirUnavailable;

/// Environment-derived inputs for path resolution.
///
/// The IO boundary: [`Self::from_env`] reads the process environment once,
/// everything downstream is pure and testable.
#[derive(Debug, Clone)]
pub struct ConfigEnv {
    home: PathBuf,
    /// Raw `$XDG_CONFIG_HOME`. The XDG spec requires an absolute path; that
    /// rule is enforced at resolution time, not at capture time, so it holds
    /// for every construction path.
    xdg_config_home: Option<PathBuf>,
    /// `$TATAMI_CONFIG_PATH`, overriding the user config location entirely.
    tatami_config_path: Option<PathBuf>,
}

impl ConfigEnv {
    pub fn from_env() -> Result<Self, HomeDirUnavailable> {
        Ok(Self {
            home: home::home_dir().ok_or(HomeDirUnavailable)?,
            xdg_config_home: env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            tatami_config_path: env::var_os("TATAMI_CONFIG_PATH").map(PathBuf::from),
        })
    }

    /// Ordered user config candidates: tatami first, worktrunk fallback.
    /// A set `TATAMI_CONFIG_PATH` short-circuits to a single candidate —
    /// the fallback rule only applies to the default locations.
    pub fn user_config_candidates(&self) -> Vec<(UserConfigSource, PathBuf)> {
        if let Some(path) = &self.tatami_config_path {
            return vec![(UserConfigSource::Tatami, path.clone())];
        }
        let dir = self
            .xdg_config_home
            .clone()
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| self.home.join(".config"));
        vec![
            (
                UserConfigSource::Tatami,
                dir.join("tatami").join("config.toml"),
            ),
            (
                UserConfigSource::WorktrunkFallback,
                dir.join("worktrunk").join("config.toml"),
            ),
        ]
    }

    /// The tatami directory inside the config root (`$XDG_CONFIG_HOME/tatami`
    /// or `~/.config/tatami`), where generated files (completions) live.
    pub fn tatami_config_dir(&self) -> PathBuf {
        let dir = self
            .xdg_config_home
            .clone()
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| self.home.join(".config"));
        dir.join("tatami")
    }

    /// The home directory the environment resolved to.
    pub fn home(&self) -> &std::path::Path {
        &self.home
    }

    /// Test-only constructor: resolution inputs without reading the process
    /// environment.
    #[cfg(test)]
    pub(crate) fn for_test(
        home: PathBuf,
        xdg_config_home: Option<PathBuf>,
        tatami_config_path: Option<PathBuf>,
    ) -> Self {
        Self {
            home,
            xdg_config_home,
            tatami_config_path,
        }
    }
}

#[cfg(test)]
mod user_config_candidates_should {
    use super::*;
    use pretty_assertions::assert_eq;

    fn env(home: &str, xdg: Option<&str>, tatami_path: Option<&str>) -> ConfigEnv {
        ConfigEnv {
            home: PathBuf::from(home),
            xdg_config_home: xdg.map(PathBuf::from),
            tatami_config_path: tatami_path.map(PathBuf::from),
        }
    }

    #[test]
    fn lists_tatami_config_before_the_worktrunk_fallback() {
        let candidates = env("/home/u", None, None).user_config_candidates();
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            candidates[0],
            (
                UserConfigSource::Tatami,
                PathBuf::from("/home/u/.config/tatami/config.toml")
            )
        );
        assert_eq!(
            candidates[1],
            (
                UserConfigSource::WorktrunkFallback,
                PathBuf::from("/home/u/.config/worktrunk/config.toml")
            )
        );
    }

    #[test]
    fn honors_an_absolute_xdg_config_home() {
        let candidates = env("/home/u", Some("/xdg"), None).user_config_candidates();
        assert_eq!(candidates[0].1, PathBuf::from("/xdg/tatami/config.toml"));
    }

    #[test]
    fn ignores_a_relative_xdg_config_home() {
        let candidates = env("/home/u", Some("relative/dir"), None).user_config_candidates();
        assert_eq!(
            candidates[0].1,
            PathBuf::from("/home/u/.config/tatami/config.toml")
        );
    }

    #[test]
    fn short_circuits_to_tatami_config_path_when_set() {
        let candidates = env("/home/u", None, Some("/custom/config.toml")).user_config_candidates();
        assert_eq!(
            candidates,
            vec![(
                UserConfigSource::Tatami,
                PathBuf::from("/custom/config.toml")
            )]
        );
    }
}
