use anyhow::Result;

use crate::{
    VERSION,
    cli::{BetCommand, ColorPolicy, Command, MarketCommand, SessionCommand},
    client::SportyClient,
    config::Settings,
    journal::Journal,
    market, orders, realtime, session, topic,
    ui::{self, Ui},
};

#[derive(Clone, Copy, Debug)]
pub struct RunOptions {
    pub json: bool,
    pub plain: bool,
    pub color: ColorPolicy,
    pub interactive: bool,
}

impl RunOptions {
    #[must_use]
    pub const fn interactive(color: ColorPolicy) -> Self {
        Self {
            json: false,
            plain: false,
            color,
            interactive: true,
        }
    }
}

/// Executes one parsed Hector command through the shared application layer.
///
/// # Errors
///
/// Returns the underlying configuration, network, protocol, journal, or UI error.
pub async fn run(command: Command, options: RunOptions) -> Result<()> {
    let ui = Ui::new(
        options.color,
        options.plain,
        options.json,
        options.interactive,
    );

    match command {
        Command::Version => {
            if options.json {
                ui::print_json(&serde_json::json!({"name": "hector", "version": VERSION}))?;
            } else if ui.is_rich() {
                ui.section("Build");
                ui.note(format!("hector {VERSION} · Rust · Linux-first"));
            } else {
                println!("hector {VERSION}");
            }
        }
        Command::Session {
            command: SessionCommand::Check,
        }
        | Command::Balance => {
            let spinner = ui.spinner("Checking the imported session…");
            let client = SportyClient::new(Settings::from_env()?)?;
            let status = session::check(&client).await;
            finish_spinner(spinner);
            let status = status?;
            if options.json {
                ui::print_json(&status)?;
            } else {
                ui.render_session(&status);
            }
        }
        Command::Market { command } => {
            let title = market_title(&command);
            let spinner = ui.spinner(&format!("Loading {title}…"));
            let client = SportyClient::new(Settings::from_env()?)?;
            let response = market::fetch(&client, &command).await;
            finish_spinner(spinner);
            let response = response?;
            if options.json {
                ui::print_json(&response)?;
            } else {
                ui.render_value(title, &response);
            }
        }
        Command::Topic { command } => {
            let value = topic::build(&command)?;
            if options.json {
                ui::print_json(&serde_json::json!({"topic": value}))?;
            } else {
                ui.render_topic(&value);
            }
        }
        Command::Stream(args) if options.interactive => ui::live::run(args).await?,
        Command::Stream(args) => realtime::stream(&args).await?,
        Command::Bet {
            command: BetCommand::Single(args),
        } => {
            let message = if args.execute {
                "Submitting one guarded order…"
            } else {
                "Building dry-run order…"
            };
            let spinner = ui.spinner(message);
            let client = SportyClient::new(Settings::from_env()?)?;
            let outcome = orders::single(&client, &args).await;
            finish_spinner(spinner);
            let outcome = outcome?;
            if options.json {
                ui::print_json(&outcome)?;
            } else if ui.is_rich() {
                ui.render_execution(&outcome);
            } else {
                println!("{}", serde_json::to_string_pretty(&outcome)?);
            }
        }
        Command::Orders { command } => {
            let records = Journal::from_env()?.load(command.journal_limit())?;
            if options.json {
                ui::print_json(&records)?;
            } else {
                ui.render_orders(&records);
            }
        }
    }

    Ok(())
}

fn finish_spinner(spinner: Option<indicatif::ProgressBar>) {
    if let Some(spinner) = spinner {
        spinner.finish_and_clear();
    }
}

fn market_title(command: &MarketCommand) -> &'static str {
    match command {
        MarketCommand::Sports(_) => "Sports",
        MarketCommand::Events(_) => "Events",
        MarketCommand::Event(_) => "Event",
        MarketCommand::MarketGroups(_) => "Market groups",
        MarketCommand::Outcomes(_) => "Outcomes",
    }
}
