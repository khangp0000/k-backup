use k_backup::backup::backup_config::BackupConfig;
use k_backup::backup::result_error::error::Error;
use k_backup::backup::result_error::AddMsg;

use clap::Parser;
use rayon::ThreadPoolBuilder;
use tracing::error;
use validator::Validate;

use std::fs::File;
use std::path::PathBuf;
use std::process::exit;

/// k-backup: Automated backup tool with encryption, compression, and retention
///
/// Creates scheduled backups of files and SQLite databases using:
/// - Cron-based scheduling
/// - XZ compression
/// - Age encryption
/// - Configurable retention policies
///
/// The tool runs as a daemon, continuously checking the cron schedule
/// and creating backups when due.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to YAML configuration file
    #[arg(short, long)]
    config: PathBuf,

    /// Run a single backup cycle and exit instead of running as a daemon
    #[arg(long)]
    once: bool,
}

fn main() {
    // Initialize structured logging for the application
    tracing_subscriber::fmt()
        .with_level(true)
        .with_file(true)
        .with_line_number(true)
        .with_thread_names(true)
        .with_thread_ids(true)
        .with_target(true)
        .init();

    let args = Args::parse();

    // Create thread pool for parallel operations during backup creation
    // Used for concurrent file processing and compression
    let thread_pool = ThreadPoolBuilder::new().build().unwrap();

    // Load, parse, and validate configuration file
    let res = File::open(&args.config)
        .map_err(Error::from)
        // Parse YAML configuration into BackupConfig struct
        .and_then(|f| {
            serde_yml::from_reader::<_, BackupConfig>(f)
                .map_err(Error::from)
                .add_msg(format!("Parse YAML config failed: {:?}", &args.config))
        })
        // Validate configuration fields (cron syntax, paths, etc.)
        .and_then(|bc| {
            bc.validate()
                .map_err(Error::from)
                .map(|_| bc)
                .add_msg(format!("Config validation failed: {:?}", &args.config))
        })
        // Run backup: once if --once flag or no cron, otherwise daemon loop
        .and_then(|bc| {
            if args.once || bc.cron().is_none() {
                bc.run_once(thread_pool.into())
            } else {
                bc.start_loop(thread_pool.into())
            }
        });

    match res {
        Ok(_) => {}
        Err(e) => {
            error!("{e}");
            exit(1);
        }
    }
}
