use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::{market::QueryParam, orders::ScaledAmount};

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

    /// Build validated realtime subscription topics.
    Topic {
        #[command(subcommand)]
        command: TopicCommand,
    },

    /// Stream live price and event updates over the public realtime socket.
    Stream(StreamArgs),

    /// Build or submit a guarded bet order.
    Bet {
        #[command(subcommand)]
        command: BetCommand,
    },

    /// Inspect locally journaled order attempts.
    Orders {
        #[command(subcommand)]
        command: OrdersCommand,
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

#[derive(Debug, Subcommand)]
pub enum TopicCommand {
    /// Build a four-field event topic.
    Event(EventTopicArgs),
    /// Build a seven-field market topic.
    Market(MarketTopicArgs),
}

#[derive(Debug, Args)]
pub struct EventTopicArgs {
    #[arg(long)]
    pub sport_id: String,
    #[arg(long)]
    pub category_id: String,
    #[arg(long)]
    pub tournament_id: String,
    #[arg(long)]
    pub event_id: String,
}

#[derive(Debug, Args)]
pub struct MarketTopicArgs {
    #[command(flatten)]
    pub event: EventTopicArgs,
    #[arg(long)]
    pub product_id: String,
    #[arg(long)]
    pub market_id: String,
    /// Market specifier, or `~` when the market has none.
    #[arg(long, default_value = "~")]
    pub specifier: String,
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

#[derive(Debug, Subcommand)]
pub enum BetCommand {
    /// Build or submit one single-selection order.
    Single(SingleBetArgs),
}

#[derive(Debug, Args)]
pub struct SingleBetArgs {
    #[arg(long)]
    pub event_id: String,
    #[arg(long)]
    pub sport_id: String,
    /// Upstream market product: typically 1 for live or 3 for prematch.
    #[arg(long)]
    pub product_id: u32,
    #[arg(long)]
    pub market_id: String,
    #[arg(long)]
    pub outcome_id: String,
    #[arg(long)]
    pub specifier: Option<String>,
    /// Decimal odds copied from the current outcome.
    #[arg(long)]
    pub odds: String,
    /// Outcome probability copied from the current outcome.
    #[arg(long)]
    pub probability: String,
    /// Stake in major currency units, with up to four decimal places.
    #[arg(long)]
    pub stake: ScaledAmount,
    /// Upstream wallet/payment type shown by the active web session.
    #[arg(long)]
    pub payment_type: u32,
    /// Actually submit the order. Without this flag Hector only prints a dry-run.
    #[arg(long)]
    pub execute: bool,
    /// Second explicit confirmation required together with `--execute`.
    #[arg(long, requires = "execute")]
    pub confirm_order: bool,
    /// Hard stake ceiling required for execution.
    #[arg(long)]
    pub max_stake: Option<ScaledAmount>,
}

#[derive(Debug, Subcommand)]
pub enum OrdersCommand {
    /// Print the newest append-only journal records.
    Journal {
        /// Maximum records to return; zero means all records.
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
}

impl OrdersCommand {
    #[must_use]
    pub const fn journal_limit(&self) -> usize {
        match self {
            Self::Journal { limit } => *limit,
        }
    }
}
