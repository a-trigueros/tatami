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
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Hello { name } => {
            println!("Hello, {name}!");
        }
    }
}
