use anyhow::Result;
use clap::Parser;
use hector_cli::{
    VERSION,
    cli::{Cli, Command},
};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Version => {
            println!("hector {VERSION}");
        }
    }

    Ok(())
}
