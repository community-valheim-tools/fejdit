use clap::{Parser, Subcommand};
use fejdit::commands;

/// Query and edit Valheim (Fejd) world and character save files.
#[derive(Parser)]
#[command(name = "fejdit", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect or edit a world (a .fwl metadata file plus its .db data file)
    World {
        #[command(subcommand)]
        cmd: commands::world::WorldCmd,
    },
    /// Inspect or edit a character (.fch file)
    #[command(alias = "char")]
    Character {
        #[command(subcommand)]
        cmd: commands::character::CharCmd,
    },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::World { cmd } => commands::world::run(cmd),
        Command::Character { cmd } => commands::character::run(cmd),
    };
    if let Err(err) = result {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
