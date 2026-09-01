use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "hector",
    version,
    about = "Low-latency SportyBet market data and guarded execution"
)]
pub struct Cli {
    /// Emit machine-readable JSON.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print build and runtime information.
    Version,

    /// Validate an imported browser session.
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },

    /// Print the account balance for the configured currency.
    Balance,
}

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Check the cookie-backed session without changing account state.
    Check,
}
