use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "hector",
    version,
    about = "Low-latency SportyBet market data and guarded execution"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print build and runtime information.
    Version,
}
