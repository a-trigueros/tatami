use tatami::{cmd, config, jj, paths};

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "tatami", version, about = "Tatami CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Print a greeting
    Hello {
        /// Name to greet
        #[arg(default_value = "world")]
        name: String,
    },
    /// List all workspaces with their working-copy status
    List,
    /// Inspect tatami configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigCommand {
    /// Print the resolved user config location, its source, and the effective
    /// workspace-path template
    Show,
}

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
        },
    }
}
