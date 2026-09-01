use clap::{Args, Parser, Subcommand, ValueEnum};

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

    /// Stream live price and event updates over the public realtime socket.
    Stream(StreamArgs),
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

#[derive(Debug, Args)]
pub struct StreamArgs {
    /// Raw caret-separated subscription topic. Repeat to subscribe to more topics.
    #[arg(long, required = true)]
    pub topic: Vec<String>,

    /// Socket.IO push routing mode.
    #[arg(long, value_enum, default_value_t = PushType::Group)]
    pub push_type: PushType,

    /// Account identifier for account-scoped MULTI streams.
    #[arg(long)]
    pub account_id: Option<String>,

    /// Engine.IO 3 WebSocket endpoint.
    #[arg(
        long,
        env = "SPORTYBET_SOCKET_URL",
        default_value = "wss://alive-ng.sportybet.com/socket.io/?EIO=3&transport=websocket"
    )]
    pub socket_url: String,

    /// Device ID sent during socket registration.
    #[arg(long, env = "SPORTYBET_DEVICE_ID")]
    pub device_id: Option<String>,

    /// Upstream product code used by the web client.
    #[arg(long, default_value_t = 7)]
    pub product_code: u32,

    /// Print unparsed Engine.IO frames.
    #[arg(long)]
    pub raw: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum PushType {
    Group,
    Multi,
    Special,
}

impl PushType {
    #[must_use]
    pub const fn as_upstream(self) -> &'static str {
        match self {
            Self::Group => "GROUP",
            Self::Multi => "MULTI",
            Self::Special => "SPECIAL",
        }
    }
}
