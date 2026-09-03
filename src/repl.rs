use std::{fs, io::IsTerminal, path::PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use console::{Style, Term};
use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};
use directories::ProjectDirs;
use rustyline::{
    CompletionType, Config, Context, EditMode, Editor, Helper,
    completion::{Completer, Pair},
    error::ReadlineError,
    highlight::Highlighter,
    hint::Hinter,
    history::DefaultHistory,
    validate::Validator,
};

use crate::{
    app::{self, RunOptions},
    cli::{
        BetCommand, Cli, ColorPolicy, Command, EventTopicArgs, MarketTopicArgs, SessionCommand,
        SingleBetArgs, TopicCommand,
    },
    client::SportyClient,
    config::Settings,
    journal::Journal,
    orders::ScaledAmount,
    session,
    ui::Ui,
};

const COMPLETIONS: &[&str] = &[
    "balance", "bet", "clear", "event", "events", "exit", "help", "history", "market", "markets",
    "orders", "outcomes", "quit", "session", "sports", "status", "stream", "topic", "version",
    "watch",
];

struct HectorHelper;

impl Helper for HectorHelper {}
impl Highlighter for HectorHelper {}
impl Validator for HectorHelper {}

impl Hinter for HectorHelper {
    type Hint = String;
}

impl Completer for HectorHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        position: usize,
        _context: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let start = line[..position]
            .rfind(char::is_whitespace)
            .map_or(0, |index| index + 1);
        let prefix = &line[start..position].to_ascii_lowercase();
        let candidates = COMPLETIONS
            .iter()
            .filter(|candidate| candidate.starts_with(prefix))
            .map(|candidate| Pair {
                display: (*candidate).to_owned(),
                replacement: (*candidate).to_owned(),
            })
            .collect();
        Ok((start, candidates))
    }
}

/// Starts Hector's persistent interactive command shell.
///
/// # Errors
///
/// Returns an error when terminal setup, history persistence, or a fatal startup operation fails.
pub async fn run(color: ColorPolicy) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("interactive mode requires a terminal; run `hector --help` for one-shot commands")
    }

    let ui = Ui::new(color, false, false, true);
    ui.banner();
    show_startup_status(ui).await;

    let config = Config::builder()
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .edit_mode(EditMode::Emacs)
        .auto_add_history(false)
        .build();
    let mut editor = Editor::<HectorHelper, DefaultHistory>::with_config(config)?;
    editor.set_helper(Some(HectorHelper));
    let history_path = history_path();
    if let Some(path) = &history_path {
        if path.exists() {
            let _ = editor.load_history(path);
        }
    }

    loop {
        let prompt = if ui.has_color() {
            "\x1b[1;36mhector\x1b[0m \x1b[2m›\x1b[0m "
        } else {
            "hector › "
        };
        match editor.readline(prompt) {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if safe_for_history(line) {
                    let _ = editor.add_history_entry(line);
                }

                match line.to_ascii_lowercase().as_str() {
                    "exit" | "quit" => break,
                    "help" | "?" => {
                        print_help();
                        continue;
                    }
                    "clear" => {
                        Term::stdout().clear_screen()?;
                        ui.banner();
                        show_startup_status(ui).await;
                        continue;
                    }
                    "history" => {
                        print_history(editor.history());
                        continue;
                    }
                    "status" => {
                        run_repl_command(
                            Command::Session {
                                command: SessionCommand::Check,
                            },
                            color,
                        )
                        .await;
                        continue;
                    }
                    "bet" => {
                        match bet_wizard() {
                            Ok(command) => run_repl_command(command, color).await,
                            Err(error) => ui.error(format!("{error:#}")),
                        }
                        continue;
                    }
                    "topic" => {
                        match topic_wizard() {
                            Ok(command) => run_repl_command(command, color).await,
                            Err(error) => ui.error(format!("{error:#}")),
                        }
                        continue;
                    }
                    _ => {}
                }

                match parse_line(line) {
                    Ok(cli) => {
                        let Some(command) = cli.command else {
                            continue;
                        };
                        let selected_color = if cli.color == ColorPolicy::Auto {
                            color
                        } else {
                            cli.color
                        };
                        if let Err(error) = app::run(
                            command,
                            RunOptions {
                                json: cli.json,
                                plain: cli.plain,
                                color: selected_color,
                                interactive: !cli.json && !cli.plain,
                            },
                        )
                        .await
                        {
                            ui.error(format!("{error:#}"));
                        }
                    }
                    Err(error) => ui.error(error),
                }
            }
            Err(ReadlineError::Interrupted) => println!("^C"),
            Err(ReadlineError::Eof) => break,
            Err(error) => return Err(error).context("interactive prompt failed"),
        }
    }

    if let Some(path) = history_path {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        editor.save_history(&path)?;
    }
    println!("{}", Style::new().dim().apply_to("Hector closed."));
    Ok(())
}

async fn run_repl_command(command: Command, color: ColorPolicy) {
    let ui = Ui::new(color, false, false, true);
    if let Err(error) = app::run(command, RunOptions::interactive(color)).await {
        ui.error(format!("{error:#}"));
    }
}

async fn show_startup_status(ui: Ui) {
    let journal_records = Journal::from_env()
        .and_then(|journal| journal.load(0))
        .map_or(0, |records| records.len());
    let settings = match Settings::from_env() {
        Ok(settings) => settings,
        Err(error) => {
            ui.startup_status(Err(&error.to_string()), journal_records);
            return;
        }
    };
    if settings.cookie.is_none() {
        ui.startup_status(Err("no imported session"), journal_records);
        return;
    }

    let spinner = ui.spinner("Checking session…");
    let status = match SportyClient::new(settings) {
        Ok(client) => session::check(&client)
            .await
            .map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    };
    if let Some(spinner) = spinner {
        spinner.finish_and_clear();
    }
    match status {
        Ok(status) => ui.startup_status(Ok(&status), journal_records),
        Err(error) => ui.startup_status(Err(&error), journal_records),
    }
}

fn parse_line(line: &str) -> Result<Cli, String> {
    let tokens = shell_words::split(line).map_err(|error| error.to_string())?;
    let tokens = expand_alias(tokens)?;
    let mut arguments = vec!["hector".to_owned()];
    arguments.extend(tokens);
    Cli::try_parse_from(arguments).map_err(|error| error.render().ansi().to_string())
}

fn expand_alias(tokens: Vec<String>) -> Result<Vec<String>, String> {
    let Some(command) = tokens.first().map(|value| value.to_ascii_lowercase()) else {
        return Ok(tokens);
    };
    let rest = &tokens[1..];
    match command.as_str() {
        "sports" => Ok(with_params(["market", "sports"], rest)),
        "events" => Ok(with_params(["market", "events"], rest)),
        "event" => event_alias("event", rest),
        "markets" => event_alias("market-groups", rest),
        "outcomes" => event_alias("outcomes", rest),
        "orders" if rest.is_empty() => Ok(vec!["orders".to_owned(), "journal".to_owned()]),
        "watch" => {
            if rest.is_empty() {
                Err("usage: watch <topic> [<topic> ...]".to_owned())
            } else {
                let mut output = vec!["stream".to_owned()];
                for topic in rest {
                    output.push("--topic".to_owned());
                    output.push(topic.clone());
                }
                Ok(output)
            }
        }
        _ => Ok(tokens),
    }
}

fn event_alias(subcommand: &str, rest: &[String]) -> Result<Vec<String>, String> {
    let Some(event_id) = rest.first() else {
        return Err(format!("usage: {subcommand} <event-id> [KEY=VALUE ...]"));
    };
    let mut output = vec![
        "market".to_owned(),
        subcommand.to_owned(),
        "--param".to_owned(),
        format!("eventId={event_id}"),
    ];
    for parameter in &rest[1..] {
        output.push("--param".to_owned());
        output.push(parameter.clone());
    }
    Ok(output)
}

fn with_params<const N: usize>(prefix: [&str; N], rest: &[String]) -> Vec<String> {
    let mut output = prefix
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    let mut index = 0;
    while index < rest.len() {
        if rest[index].starts_with('-') {
            output.push(rest[index].clone());
            if rest[index] == "--param" && index + 1 < rest.len() {
                index += 1;
                output.push(rest[index].clone());
            }
        } else {
            output.push("--param".to_owned());
            output.push(rest[index].clone());
        }
        index += 1;
    }
    output
}

fn bet_wizard() -> Result<Command> {
    let theme = ColorfulTheme::default();
    println!();
    println!(
        "{}",
        Style::new().cyan().bold().apply_to("GUIDED SINGLE BET")
    );
    let event_id = Input::<String>::with_theme(&theme)
        .with_prompt("Event ID")
        .interact_text()?;
    let sport_id = Input::<String>::with_theme(&theme)
        .with_prompt("Sport ID")
        .default("sr:sport:1".to_owned())
        .interact_text()?;
    let product_id = [3_u32, 1_u32][Select::with_theme(&theme)
        .with_prompt("Market type")
        .items(&["Prematch", "Live"])
        .default(0)
        .interact()?];
    let market_id = Input::<String>::with_theme(&theme)
        .with_prompt("Market ID")
        .interact_text()?;
    let outcome_id = Input::<String>::with_theme(&theme)
        .with_prompt("Outcome ID")
        .interact_text()?;
    let specifier = Input::<String>::with_theme(&theme)
        .with_prompt("Specifier (blank for none)")
        .allow_empty(true)
        .interact_text()?;
    let odds = Input::<String>::with_theme(&theme)
        .with_prompt("Decimal odds")
        .interact_text()?;
    let probability = Input::<String>::with_theme(&theme)
        .with_prompt("Probability")
        .interact_text()?;
    let stake_text = Input::<String>::with_theme(&theme)
        .with_prompt("Stake")
        .interact_text()?;
    let stake: ScaledAmount = stake_text.parse()?;
    let payment_type = Input::<u32>::with_theme(&theme)
        .with_prompt("Payment type")
        .default(0)
        .interact_text()?;

    let live = Select::with_theme(&theme)
        .with_prompt("Order mode")
        .items(&["Dry run — preview only", "LIVE — submit one wager"])
        .default(0)
        .interact()?
        == 1;
    let mut execute = false;
    let mut confirm_order = false;
    let mut max_stake = None;
    if live {
        println!(
            "{}",
            Style::new()
                .red()
                .bold()
                .apply_to(format!("LIVE ORDER · MAXIMUM LOSS {stake}"))
        );
        if !Confirm::with_theme(&theme)
            .with_prompt("Continue to the final execution guard?")
            .default(false)
            .interact()?
        {
            bail!("live execution cancelled");
        }
        let required = format!("PLACE {stake}");
        let confirmation = Input::<String>::with_theme(&theme)
            .with_prompt(format!("Type {required}"))
            .interact_text()?;
        if confirmation != required {
            bail!("confirmation did not match; no wager was submitted");
        }
        execute = true;
        confirm_order = true;
        max_stake = Some(stake);
    }

    Ok(Command::Bet {
        command: BetCommand::Single(SingleBetArgs {
            event_id,
            sport_id,
            product_id,
            market_id,
            outcome_id,
            specifier: (!specifier.is_empty()).then_some(specifier),
            odds,
            probability,
            stake,
            payment_type,
            execute,
            confirm_order,
            max_stake,
        }),
    })
}

fn topic_wizard() -> Result<Command> {
    let theme = ColorfulTheme::default();
    let kind = Select::with_theme(&theme)
        .with_prompt("Topic type")
        .items(&["Market", "Event"])
        .default(0)
        .interact()?;
    let event = EventTopicArgs {
        sport_id: Input::with_theme(&theme)
            .with_prompt("Sport ID")
            .default("1".to_owned())
            .interact_text()?,
        category_id: Input::with_theme(&theme)
            .with_prompt("Category ID")
            .interact_text()?,
        tournament_id: Input::with_theme(&theme)
            .with_prompt("Tournament ID")
            .interact_text()?,
        event_id: Input::with_theme(&theme)
            .with_prompt("Event ID")
            .interact_text()?,
    };
    let command = if kind == 1 {
        TopicCommand::Event(event)
    } else {
        TopicCommand::Market(MarketTopicArgs {
            event,
            product_id: Input::with_theme(&theme)
                .with_prompt("Product ID")
                .default("3".to_owned())
                .interact_text()?,
            market_id: Input::with_theme(&theme)
                .with_prompt("Market ID")
                .interact_text()?,
            specifier: Input::with_theme(&theme)
                .with_prompt("Specifier")
                .default("~".to_owned())
                .interact_text()?,
        })
    };
    Ok(Command::Topic { command })
}

fn history_path() -> Option<PathBuf> {
    ProjectDirs::from("com", "sylva", "hector").map(|directories| {
        directories
            .state_dir()
            .unwrap_or_else(|| directories.data_local_dir())
            .join("history")
    })
}

fn safe_for_history(line: &str) -> bool {
    let lowercase = line.to_ascii_lowercase();
    ![
        "cookie",
        "accesstoken",
        "refreshtoken",
        "password",
        "secret",
    ]
    .iter()
    .any(|secret| lowercase.contains(secret))
}

fn print_history(history: &DefaultHistory) {
    for (index, entry) in history.iter().enumerate() {
        println!("{:>4}  {entry}", index + 1);
    }
}

fn print_help() {
    println!();
    println!("{}", Style::new().cyan().bold().apply_to("COMMANDS"));
    println!(
        "  {:<22} account health and available balance",
        "status · balance"
    );
    println!("  {:<22} browse public market data", "sports · events");
    println!("  {:<22} inspect one fixture", "event <event-id>");
    println!("  {:<22} list its market groups", "markets <event-id>");
    println!("  {:<22} fetch current outcomes", "outcomes <event-id>");
    println!("  {:<22} build a validated subscription", "topic");
    println!("  {:<22} open the realtime TUI", "watch <topic>");
    println!("  {:<22} guided dry-run/live order", "bet");
    println!("  {:<22} inspect durable attempts", "orders");
    println!("  {:<22} prompt utilities", "history · clear · exit");
    println!();
    println!(
        "{}",
        Style::new().dim().apply_to(
            "Raw CLI syntax also works here. Add --json or --plain for machine-readable output."
        )
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_market_shortcuts() {
        assert_eq!(
            expand_alias(vec!["event".to_owned(), "sr:match:42".to_owned()]).unwrap(),
            ["market", "event", "--param", "eventId=sr:match:42"]
        );
        assert_eq!(
            expand_alias(vec!["events".to_owned(), "timeline=0".to_owned()]).unwrap(),
            ["market", "events", "--param", "timeline=0"]
        );
    }

    #[test]
    fn expands_watch_topics() {
        assert_eq!(
            expand_alias(vec!["watch".to_owned(), "a^b^c".to_owned()]).unwrap(),
            ["stream", "--topic", "a^b^c"]
        );
    }

    #[test]
    fn refuses_sensitive_history() {
        assert!(!safe_for_history("set SPORTYBET_COOKIE=private"));
        assert!(safe_for_history("events timeline=0"));
    }

    #[test]
    fn parses_short_event_command() {
        let cli = parse_line("event sr:match:42").unwrap();
        assert!(matches!(cli.command, Some(Command::Market { .. })));
    }

    #[test]
    fn supplies_all_topics_to_watch() {
        let cli = parse_line("watch 'one^topic' 'two^topic'").unwrap();
        let Some(Command::Stream(args)) = cli.command else {
            panic!("expected stream command");
        };
        assert_eq!(args.topic, ["one^topic", "two^topic"]);
        assert!(matches!(args.push_type, crate::cli::PushType::Group));
    }
}
