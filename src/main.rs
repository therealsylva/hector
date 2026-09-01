use anyhow::Result;
use clap::Parser;
use hector_cli::{
    VERSION,
    cli::{Cli, Command, SessionCommand},
    client::SportyClient,
    config::Settings,
    market, orders, realtime, session,
};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Version => {
            println!("hector {VERSION}");
        }
        Command::Session {
            command: SessionCommand::Check,
        }
        | Command::Balance => {
            let client = SportyClient::new(Settings::from_env()?)?;
            let status = session::check(&client).await?;
            if cli.json {
                println!("{}", serde_json::to_string(&status)?);
            } else {
                println!("session: authenticated");
                println!("balance: {} {}", status.currency, status.available_balance);
                if status.available_coins != "0.0000" {
                    println!("coins: {}", status.available_coins);
                }
            }
        }
        Command::Market { command } => {
            let client = SportyClient::new(Settings::from_env()?)?;
            let response = market::fetch(&client, &command).await?;
            if cli.json {
                println!("{}", serde_json::to_string(&response)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&response)?);
            }
        }
        Command::Stream(args) => realtime::stream(&args).await?,
        Command::Bet {
            command: hector_cli::cli::BetCommand::Single(args),
        } => {
            let client = SportyClient::new(Settings::from_env()?)?;
            let outcome = orders::single(&client, &args).await?;
            if cli.json {
                println!("{}", serde_json::to_string(&outcome)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&outcome)?);
            }
        }
    }

    Ok(())
}
