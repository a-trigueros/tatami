use clap::Parser;
use tatami::cli::{Cli, Command, ConfigCommand, ShellCommand};
use tatami::{approvals, cmd, config, hook, jj, paths, state};

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Hello { name } => {
            println!("Hello, {name}!");
        }
        Command::List => {
            let dir = std::env::current_dir().unwrap_or_else(|error| {
                eprintln!("tatami: cannot determine the current directory: {error}");
                std::process::exit(1);
            });
            cmd::list::run(&jj::CliJj::at(dir)).unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
        }
        Command::Switch { name, create } => {
            let dir = std::env::current_dir().unwrap_or_else(|error| {
                eprintln!("tatami: cannot determine the current directory: {error}");
                std::process::exit(1);
            });
            let paths_env = paths::ConfigEnv::from_env().unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let user_config = config::load_user_config(&paths_env).unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let project_config = config::load_project_config(&dir).unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let approvals = approvals::Approvals::load(&paths_env).unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let project_id = state::host_root(&dir)
                .unwrap_or_else(|error| {
                    eprintln!("tatami: {error}");
                    std::process::exit(1);
                })
                .display()
                .to_string();
            let mut gate = approvals::InteractiveGate::interactive(approvals, project_id, cli.yes);
            let hooks = hook::Hooks {
                user: Some(&user_config.config),
                project: Some(&project_config.config),
            };
            let adapter = jj::CliJj::at(dir.clone());
            let state = state::FileState::for_workspace(&dir).unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let args = cmd::switch::SwitchArgs {
                name: name.as_deref(),
                create,
                hooks: &hooks,
                current_root: &dir,
                home: paths_env.home(),
            };
            let destination = cmd::switch::run(
                &adapter,
                args,
                &state,
                // Handles, not locks: the interactive approval gate reads the
                // same streams and Stdin's internal lock is not reentrant.
                &mut std::io::stderr(),
                &mut std::io::BufReader::new(std::io::stdin()),
                &mut gate,
            )
            .unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            println!("{}", destination.display());
        }
        Command::Remove {
            name,
            delete_bookmark,
        } => {
            let dir = std::env::current_dir().unwrap_or_else(|error| {
                eprintln!("tatami: cannot determine the current directory: {error}");
                std::process::exit(1);
            });
            let paths_env = paths::ConfigEnv::from_env().unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let user_config = config::load_user_config(&paths_env).unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let project_config = config::load_project_config(&dir).unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let approvals = approvals::Approvals::load(&paths_env).unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let project_id = state::host_root(&dir)
                .unwrap_or_else(|error| {
                    eprintln!("tatami: {error}");
                    std::process::exit(1);
                })
                .display()
                .to_string();
            let mut gate = approvals::InteractiveGate::interactive(approvals, project_id, cli.yes);
            let hooks = hook::Hooks {
                user: Some(&user_config.config),
                project: Some(&project_config.config),
            };
            let state_dir = state::state_dir(&dir).unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let report = cmd::remove::run(
                &jj::CliJj::at(dir.clone()),
                cmd::remove::RemoveArgs {
                    name: &name,
                    delete_bookmark,
                    now_millis: now_millis(),
                    state_dir: &state_dir,
                    hooks: &hooks,
                    caller_root: &dir,
                },
                &mut std::io::stderr(),
                &mut gate,
            )
            .unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            println!("removed workspace `{}`", report.name);
            if report.bookmark_deleted {
                println!("bookmark deleted");
            }
            match report.directory {
                cmd::remove::DirectoryOutcome::Trashed(path) => {
                    println!("directory moved to trash: {}", path.display());
                }
                cmd::remove::DirectoryOutcome::AlreadyGone => {}
            }
        }
        Command::Rebase { target } => {
            let adapter = jj::CliJj::at(std::env::current_dir().unwrap_or_else(|error| {
                eprintln!("tatami: cannot determine the current directory: {error}");
                std::process::exit(1);
            }));
            let outcome = cmd::rebase::run(&adapter, target.as_deref()).unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            println!(
                "rebased {} commit(s) onto `{}`",
                outcome.commits, outcome.target
            );
        }
        Command::Merge { target } => {
            let dir = std::env::current_dir().unwrap_or_else(|error| {
                eprintln!("tatami: cannot determine the current directory: {error}");
                std::process::exit(1);
            });
            let paths_env = paths::ConfigEnv::from_env().unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let user_config = config::load_user_config(&paths_env).unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let project_config = config::load_project_config(&dir).unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let approvals = approvals::Approvals::load(&paths_env).unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let project_id = state::host_root(&dir)
                .unwrap_or_else(|error| {
                    eprintln!("tatami: {error}");
                    std::process::exit(1);
                })
                .display()
                .to_string();
            let mut gate = approvals::InteractiveGate::interactive(approvals, project_id, cli.yes);
            let hooks = hook::Hooks {
                user: Some(&user_config.config),
                project: Some(&project_config.config),
            };
            let outcome = cmd::merge::run(
                &jj::CliJj::at(dir.clone()),
                target.as_deref(),
                &hooks,
                &mut gate,
                &dir,
                &mut std::io::stderr(),
            )
            .unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            println!(
                "merged {} commit(s) into `{}`{}",
                outcome.commits,
                outcome.target,
                if outcome.rebased {
                    " (rebased)"
                } else {
                    " (fast-forward)"
                }
            );
        }
        Command::Config { command } => match command {
            ConfigCommand::Show => {
                let env = paths::ConfigEnv::from_env().unwrap_or_else(|error| {
                    eprintln!("tatami: {error}");
                    std::process::exit(1);
                });
                config::config_show(&env).unwrap_or_else(|error| {
                    eprintln!("tatami: {error}");
                    std::process::exit(1);
                });
            }
            ConfigCommand::Shell { command } => match command {
                ShellCommand::Install { shell } => {
                    cmd::shell::install(&shell_env(), shell.as_deref()).unwrap_or_else(|error| {
                        eprintln!("tatami: {error}");
                        std::process::exit(1);
                    });
                }
                ShellCommand::Uninstall { shell } => {
                    cmd::shell::uninstall(&shell_env(), shell.as_deref()).unwrap_or_else(|error| {
                        eprintln!("tatami: {error}");
                        std::process::exit(1);
                    });
                }
                ShellCommand::Completion { shell } => {
                    print!(
                        "{}",
                        cmd::shell::completion(&shell).unwrap_or_else(|error| {
                            eprintln!("tatami: {error}");
                            std::process::exit(1);
                        })
                    );
                }
            },
        },
    }
}

fn shell_env() -> cmd::shell::ShellEnv {
    let config = paths::ConfigEnv::from_env().unwrap_or_else(|error| {
        eprintln!("tatami: {error}");
        std::process::exit(1);
    });
    cmd::shell::ShellEnv::from_config_env(&config, std::env::var("SHELL").ok())
}

/// The injected clock for trash naming. One read, at the boundary.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
