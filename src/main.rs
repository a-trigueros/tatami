use clap::Parser;
use tatami::cli::{
    ApprovalsCommand, Cli, Command, ConfigCommand, ShellCommand, StateCommand, StepCommand,
};
use tatami::{approvals, cmd, config, hook, jj, paths, state};

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::List { format } => {
            let dir = std::env::current_dir().unwrap_or_else(|error| {
                eprintln!("tatami: cannot determine the current directory: {error}");
                std::process::exit(1);
            });
            cmd::list::run(&jj::CliJj::at(dir), format.into(), &mut std::io::stdout())
                .unwrap_or_else(|error| {
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
            let state_dir = state::state_dir(&dir).unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            let args = cmd::switch::SwitchArgs {
                name: name.as_deref(),
                create,
                hooks: &hooks,
                current_root: &dir,
                home: paths_env.home(),
                logs_dir: Some(&state_dir.join("logs")),
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
            keep_bookmark,
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
                    // Deleted by default (worktrunk parity: delete-branch).
                    delete_bookmark: !keep_bookmark,
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
        Command::Merge { target, remove } => {
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
            let outcome = cmd::merge::run(
                &jj::CliJj::at(dir.clone()),
                cmd::merge::MergeArgs {
                    target: target.as_deref(),
                    remove_current: remove,
                    hooks: &hooks,
                    caller_root: &dir,
                    logs_dir: Some(&state_dir.join("logs")),
                    state_dir: &state_dir,
                    now_millis: now_millis(),
                },
                &mut gate,
                &mut std::io::stderr(),
            )
            .unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
            if let Some(landing) = outcome.landing {
                println!("{}", landing.display());
            }
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
        Command::Step { command } => {
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
            match command {
                StepCommand::Commit { message } => {
                    let state_dir = state::state_dir(&dir).unwrap_or_else(|error| {
                        eprintln!("tatami: {error}");
                        std::process::exit(1);
                    });
                    cmd::step::commit(
                        &adapter,
                        &message,
                        &hooks,
                        &mut gate,
                        &dir,
                        Some(&state_dir.join("logs")),
                        &mut std::io::stderr(),
                    )
                    .unwrap_or_else(|error| {
                        eprintln!("tatami: {error}");
                        std::process::exit(1);
                    });
                }
                StepCommand::Squash => {
                    cmd::step::squash(&adapter).unwrap_or_else(|error| {
                        eprintln!("tatami: {error}");
                        std::process::exit(1);
                    });
                }
                StepCommand::Rebase { target } => {
                    let outcome =
                        cmd::step::rebase(&adapter, target.as_deref()).unwrap_or_else(|error| {
                            eprintln!("tatami: {error}");
                            std::process::exit(1);
                        });
                    println!(
                        "rebased {} commit(s) onto `{}`",
                        outcome.commits, outcome.target
                    );
                }
                StepCommand::Push => {
                    let bookmark = cmd::step::push(&adapter, None).unwrap_or_else(|error| {
                        eprintln!("tatami: {error}");
                        std::process::exit(1);
                    });
                    println!("pushed `{bookmark}`");
                }
                StepCommand::Diff => {
                    let diff = cmd::step::diff(&adapter).unwrap_or_else(|error| {
                        eprintln!("tatami: {error}");
                        std::process::exit(1);
                    });
                    print!("{diff}");
                }
                StepCommand::ForEach { command } => {
                    let outcomes = cmd::step::for_each(&adapter, &command, &mut std::io::stderr())
                        .unwrap_or_else(|error| {
                            eprintln!("tatami: {error}");
                            std::process::exit(1);
                        });
                    for outcome in &outcomes {
                        match &outcome.failed {
                            Some(reason) => println!("{}: FAILED ({})", outcome.workspace, reason),
                            None => println!("{}: ok", outcome.workspace),
                        }
                    }
                    if outcomes.iter().any(|outcome| outcome.failed.is_some()) {
                        std::process::exit(1);
                    }
                }
                StepCommand::CopyIgnored { from, to, force } => {
                    let report =
                        cmd::step::copy_ignored(&adapter, from.as_deref(), to.as_deref(), force)
                            .unwrap_or_else(|error| {
                                eprintln!("tatami: {error}");
                                std::process::exit(1);
                            });
                    for copied in &report.copied {
                        println!("copied {}", copied.display());
                    }
                    for skipped in &report.skipped_existing {
                        println!(
                            "skipped (exists): {} — use --force to overwrite",
                            skipped.display()
                        );
                    }
                }
                StepCommand::Prune => {
                    let state_dir = state::state_dir(&dir).unwrap_or_else(|error| {
                        eprintln!("tatami: {error}");
                        std::process::exit(1);
                    });
                    let pruned = cmd::step::prune(
                        &adapter,
                        &hooks,
                        &mut gate,
                        &state_dir,
                        &dir,
                        now_millis(),
                        &mut std::io::stderr(),
                    )
                    .unwrap_or_else(|error| {
                        eprintln!("tatami: {error}");
                        std::process::exit(1);
                    });
                    for workspace in &pruned {
                        println!("pruned `{}`", workspace.name);
                    }
                }
            }
        }
        Command::Unknown(args) => {
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

            let (name, rest) = args.split_first().unwrap_or_else(|| {
                eprintln!("tatami: no alias name given");
                std::process::exit(1);
            });
            let raw_args: Vec<String> = rest
                .iter()
                .map(|value| value.to_string_lossy().to_string())
                .collect();
            cmd::alias::run(
                &jj::CliJj::at(dir.clone()),
                &name.to_string_lossy(),
                &raw_args,
                &user_config.config,
                &project_config.config,
                &mut gate,
                &dir,
            )
            .unwrap_or_else(|error| {
                eprintln!("tatami: {error}");
                std::process::exit(1);
            });
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
                ShellCommand::Init { shell } => {
                    print!(
                        "{}",
                        cmd::shell::init(&shell_env(), shell.as_deref()).unwrap_or_else(|error| {
                            eprintln!("tatami: {error}");
                            std::process::exit(1);
                        })
                    );
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
            ConfigCommand::State { command } => match command {
                StateCommand::Show => {
                    let dir = std::env::current_dir().unwrap_or_else(|error| {
                        eprintln!("tatami: cannot determine the current directory: {error}");
                        std::process::exit(1);
                    });
                    let state_dir = state::state_dir(&dir).unwrap_or_else(|error| {
                        eprintln!("tatami: {error}");
                        std::process::exit(1);
                    });
                    println!("state dir: {}", state_dir.display());
                    let trash = state::trash_entries(&state_dir).unwrap_or_else(|error| {
                        eprintln!("tatami: {error}");
                        std::process::exit(1);
                    });
                    if trash.is_empty() {
                        println!("trash: empty");
                    }
                    for entry in trash {
                        println!("trash: {}", entry.display());
                    }
                }
                StateCommand::Clear { force } => {
                    let dir = std::env::current_dir().unwrap_or_else(|error| {
                        eprintln!("tatami: cannot determine the current directory: {error}");
                        std::process::exit(1);
                    });
                    let state_dir = state::state_dir(&dir).unwrap_or_else(|error| {
                        eprintln!("tatami: {error}");
                        std::process::exit(1);
                    });
                    let trash = state::trash_entries(&state_dir).unwrap_or_else(|error| {
                        eprintln!("tatami: {error}");
                        std::process::exit(1);
                    });
                    for entry in &trash {
                        println!("would remove: {}", entry.display());
                    }
                    if trash.is_empty() {
                        println!("trash: empty");
                    }
                    if !force {
                        if !trash.is_empty() {
                            println!("run with --force to actually delete");
                        }
                        return;
                    }
                    state::clear_trash(&state_dir).unwrap_or_else(|error| {
                        eprintln!("tatami: {error}");
                        std::process::exit(1);
                    });
                    println!("trash emptied");
                }
            },
            ConfigCommand::Approvals { command } => match command {
                ApprovalsCommand::Add {
                    phase,
                    command: hook_command,
                } => {
                    let phase = hook::HookPhase::parse(&phase).unwrap_or_else(|| {
                        eprintln!("tatami: unknown phase `{phase}`");
                        std::process::exit(1);
                    });
                    let joined = hook_command.join(" ");
                    if joined.trim().is_empty() {
                        eprintln!("tatami: a hook command is required");
                        std::process::exit(1);
                    }
                    let dir = std::env::current_dir().unwrap_or_else(|error| {
                        eprintln!("tatami: cannot determine the current directory: {error}");
                        std::process::exit(1);
                    });
                    let paths_env = paths::ConfigEnv::from_env().unwrap_or_else(|error| {
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
                    let mut approvals =
                        approvals::Approvals::load(&paths_env).unwrap_or_else(|error| {
                            eprintln!("tatami: {error}");
                            std::process::exit(1);
                        });
                    approvals
                        .approve(&project_id, phase.config_key(), &joined)
                        .unwrap_or_else(|error| {
                            eprintln!("tatami: {error}");
                            std::process::exit(1);
                        });
                    println!(
                        "approved `{joined}` for {} in {project_id}",
                        phase.config_key()
                    );
                }
            },
        },
    }
}

fn shell_env() -> cmd::shell::ShellEnv {
    cmd::shell::ShellEnv {
        env_shell: std::env::var("SHELL").ok(),
    }
}

/// The injected clock for trash naming. One read, at the boundary.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
