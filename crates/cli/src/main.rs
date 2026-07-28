use clap::Parser;

#[derive(Parser)]
#[command(name = "work", version, about = "Isolated multi-context session manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// placeholder so `work --version` builds; real commands land in Task 1.8
    Version,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Version) | None => println!("work {}", env!("CARGO_PKG_VERSION")),
    }
    Ok(())
}
