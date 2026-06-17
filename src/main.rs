mod config;
mod daemon;
mod email;
mod error;
mod scrub;
mod status;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::config::{load_config, resolve_config_path};
use crate::daemon::run_daemon;
use crate::error::AppError;
use crate::scrub::execute_scrub;
use crate::status::execute_status;

#[derive(Parser, Debug)]
#[command(name = "zfshealth")]
#[command(about = "ZFS health monitoring with scrub scheduling")]
struct Args {
    #[arg(long, global = true, help = "Path to configuration file")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<CommandMode>,
}

#[derive(Subcommand, Debug, Clone)]
enum CommandMode {
    Run(RunCommand),
    Daemon,
}

#[derive(Parser, Debug, Clone, Copy)]
struct RunCommand {
    #[command(subcommand)]
    mode: RunMode,
}

#[derive(Subcommand, Debug, Clone, Copy)]
enum RunMode {
    Scrub,
    Status,
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

async fn run() -> Result<(), AppError> {
    let args = Args::parse();
    let config_path = resolve_config_path(args.config)?;
    let command = args.command.unwrap_or(CommandMode::Run(RunCommand {
        mode: RunMode::Scrub,
    }));

    match command {
        CommandMode::Run(run_command) => {
            let config = load_config(config_path.as_ref()).await?;
            match run_command.mode {
                RunMode::Scrub => execute_scrub(config.email).await,
                RunMode::Status => execute_status(config.email).await,
            }
        }
        CommandMode::Daemon => run_daemon(
            config_path.ok_or_else(|| {
                AppError::ConfigFile(
                    "Daemon mode requires a configuration file. Pass --config or create the default config file."
                        .to_string(),
                )
            })?,
        )
        .await,
    }
}
