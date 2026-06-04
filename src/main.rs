mod config;
mod cycle;
mod error;
mod notifications;
mod pipeline;
mod retention;
mod scheduler;

use clap::Parser;
use error::{Context, Error};
use std::fs::File;
use std::path::PathBuf;
use std::process::exit;
use std::sync::Arc;

/// k-backup: Automated backup with encryption, compression, and retention
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Path to YAML configuration file
    #[arg(short, long)]
    config: PathBuf,
    /// Run a single backup cycle and exit
    #[arg(long)]
    once: bool,
}

fn main() {
    tracing_subscriber::fmt()
        .with_level(true)
        .with_file(true)
        .with_line_number(true)
        .with_thread_names(true)
        .with_target(true)
        .init();

    let args = Args::parse();

    let result = run(args);
    if let Err(e) = result {
        tracing::error!("{}", e);
        exit(1);
    }
}

fn run(args: Args) -> error::Result<()> {
    let config: Arc<config::BackupConfig> = {
        let file = File::open(&args.config)
            .map_err(Error::from)
            .context(format!("Failed to open config: {:?}", args.config))?;
        serde_saphyr::from_reader(file)
            .map_err(|e| Error::from(error::ConfigError::from(e)))
            .context("Failed to parse config")?
    };

    config.validate()?;

    let pool = Arc::new(
        rayon::ThreadPoolBuilder::new()
            .build()
            .map_err(|e| Error::from(std::io::Error::other(e)))?,
    );

    if args.once || config.cron.is_none() {
        scheduler::run_once(config, pool)
    } else {
        scheduler::start_loop(config, pool)
    }
}
