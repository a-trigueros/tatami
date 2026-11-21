//! User config loading: schema (see [`schema`]) and resolution order.
//!
//! Resolution: tatami's own user config first, worktrunk's as fallback. The
//! full layering (project config, env vars, `--config-set`) lands with the
//! milestones that consume those layers.

pub mod schema;

pub use schema::UserConfig;

use std::fs;
use std::path::PathBuf;

use thiserror::Error;

use crate::paths::{ConfigEnv, UserConfigSource};

/// The user config as loaded: which file won, where it lives, and its parsed
/// content. `source: None` means no config file exists anywhere — a missing
/// config is normal (defaults apply), a broken one is not.
#[derive(Debug)]
pub struct LoadedUserConfig {
    pub source: Option<UserConfigSource>,
    pub path: Option<PathBuf>,
    pub config: UserConfig,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read config file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot parse config file {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

/// Loads the first existing user config candidate. A file that exists but
/// fails to parse is an error — never a silent fallback to the next candidate.
pub fn load_user_config(env: &ConfigEnv) -> Result<LoadedUserConfig, ConfigError> {
    for (source, path) in env.user_config_candidates() {
        if !path.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: path.clone(),
            source,
        })?;
        let config = toml::from_str(&raw).map_err(|source| ConfigError::Parse {
            path: path.clone(),
            source,
        })?;
        return Ok(LoadedUserConfig {
            source: Some(source),
            path: Some(path),
            config,
        });
    }
    Ok(LoadedUserConfig {
        source: None,
        path: None,
        config: UserConfig::default(),
    })
}

/// `tt config show` — prints the resolved user config location, its source,
/// and the effective workspace location template.
pub fn config_show(env: &ConfigEnv) -> Result<(), ConfigError> {
    let loaded = load_user_config(env)?;
    match (&loaded.source, &loaded.path) {
        (Some(source), Some(path)) => {
            println!("user config: {} ({})", path.display(), source.label())
        }
        (None, _) => println!("user config: none (defaults)"),
        (Some(_), None) => unreachable!("a loaded config always knows its file"),
    }
    match loaded.config.workspace_path() {
        Some(template) => println!("workspace-path: {template}"),
        None => println!("workspace-path: not set"),
    }
    Ok(())
}

#[cfg(test)]
mod load_user_config_should {
    use super::*;
    use crate::paths::UserConfigSource as Source;
    use pretty_assertions::{assert_eq, assert_str_eq};
    use std::path::Path;

    fn env_for(home: &Path) -> ConfigEnv {
        ConfigEnv::for_test(home.to_path_buf(), None, None)
    }

    fn write(home: &Path, tool: &str, content: &str) {
        let dir = home.join(".config").join(tool);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.toml"), content).unwrap();
    }

    #[test]
    fn loads_the_tatami_config_when_it_exists() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            "tatami",
            "workspace-path = \".ws/{{ workspace }}\"\n",
        );
        write(
            home.path(),
            "worktrunk",
            "worktree-path = \".wt/{{ branch }}\"\n",
        );

        let loaded = load_user_config(&env_for(home.path())).unwrap();

        assert_eq!(loaded.source, Some(Source::Tatami));
        assert_eq!(
            loaded.path.as_deref(),
            Some(
                home.path()
                    .join(".config")
                    .join("tatami")
                    .join("config.toml")
                    .as_path()
            )
        );
        assert_str_eq!(
            loaded.config.workspace_path().unwrap(),
            ".ws/{{ workspace }}"
        );
    }

    #[test]
    fn falls_back_to_the_worktrunk_config_when_tatami_is_absent() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            "worktrunk",
            "worktree-path = \".wt/{{ branch }}\"\n",
        );

        let loaded = load_user_config(&env_for(home.path())).unwrap();

        assert_eq!(loaded.source, Some(Source::WorktrunkFallback));
        assert_str_eq!(loaded.config.workspace_path().unwrap(), ".wt/{{ branch }}");
    }

    #[test]
    fn uses_defaults_when_no_config_exists() {
        let home = tempfile::tempdir().unwrap();

        let loaded = load_user_config(&env_for(home.path())).unwrap();

        assert_eq!(loaded.source, None);
        assert_eq!(loaded.path, None);
        assert_eq!(loaded.config.workspace_path(), None);
    }

    #[test]
    fn fails_rather_than_falling_back_when_the_tatami_config_is_invalid() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "tatami", "[[[ not toml ]]\n");
        write(
            home.path(),
            "worktrunk",
            "worktree-path = \".wt/{{ branch }}\"\n",
        );

        let error = load_user_config(&env_for(home.path())).unwrap_err();

        assert!(
            matches!(error, ConfigError::Parse { .. }),
            "expected a parse error, got: {error:?}"
        );
    }
}
