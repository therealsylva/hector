use clap::{Args, Parser, Subcommand};

use crate::market::QueryParam;

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

    /// Query `SportyBet`'s public fixtures and markets.
    Market {
        #[command(subcommand)]
        command: MarketCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Check the cookie-backed session without changing account state.
    Check,
}

#[derive(Debug, Subcommand)]
pub enum MarketCommand {
    /// List available sports.
    Sports(MarketQueryArgs),
    /// List live or prematch events.
    Events(MarketQueryArgs),
    /// Fetch one event and its current markets.
    Event(MarketQueryArgs),
    /// List the market groups available for an event.
    MarketGroups(MarketQueryArgs),
    /// Fetch outcomes and current prices.
    Outcomes(MarketQueryArgs),
}

impl MarketCommand {
    #[must_use]
    pub fn query(&self) -> &[QueryParam] {
        match self {
            Self::Sports(args)
            | Self::Events(args)
            | Self::Event(args)
            | Self::MarketGroups(args)
            | Self::Outcomes(args) => &args.params,
        }
    }
}

#[derive(Debug, Args)]
pub struct MarketQueryArgs {
    /// Query value passed through to the upstream endpoint. Repeat as needed.
    #[arg(long = "param", value_name = "KEY=VALUE")]
    pub params: Vec<QueryParam>,
}
