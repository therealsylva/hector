use std::io::IsTerminal;

use anyhow::{Result, bail};
use clap::Parser;
use hector_cli::{
    app::{self, RunOptions},
    cli::Cli,
    repl,
};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        if cli.json || cli.plain || !std::io::stdin().is_terminal() {
            bail!("no command supplied; run `hector --help` or launch `hector` in a terminal")
        }
        return repl::run(cli.color).await;
    };
    app::run(
        command,
        RunOptions {
            json: cli.json,
            plain: cli.plain,
            color: cli.color,
            interactive: false,
        },
    )
    .await
}
