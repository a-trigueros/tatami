use clap::Parser;
use tatami::cli::{Cli, Command, ConfigCommand, ShellCommand};
use tatami::{cmd, config, jj, paths, state};

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
            let adapter = jj::CliJj::at(dir.clone());
            let state = state::FileState::for_workspace(&dir).unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let args = cmd::switch::SwitchArgs {
                name: name.as_deref(),
                create,
                workspace_template: user_config.config.workspace_path(),
                current_root: &dir,
                home: paths_env.home(),
            };
            let destination = cmd::switch::run(
                &adapter,
                args,
                &state,
                &mut std::io::stderr().lock(),
                &mut std::io::stdin().lock(),
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
            let state_dir = state::state_dir(&dir).unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let report = cmd::remove::run(
                &jj::CliJj::at(dir),
                cmd::remove::RemoveArgs {
                    name: &name,
                    delete_bookmark,
                    now_millis: now_millis(),
                    state_dir: &state_dir,
                },
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
