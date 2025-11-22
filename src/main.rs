use clap::Parser;
use k_backup::backup::backup_config::BackupConfig;
use k_backup::backup::result_error::error::Error;
use k_backup::backup::result_error::AddMsg;
use rayon::ThreadPoolBuilder;
use std::fs::File;
use std::path::PathBuf;
use std::process::exit;
use tracing::error;
use validator::Validate;

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
    ///
    /// The config file specifies:
    /// - Backup schedule (cron expression)
    /// - Source files/directories to backup
    /// - Output directory and naming
    /// - Compression and encryption settings
    /// - Retention policy for old backups
    #[arg(short, long)]
    config: PathBuf,
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
        // Start the main backup daemon loop
        // This runs forever, checking cron schedule and creating backups
        .and_then(|bc| bc.start_loop(thread_pool.into()));

    match res {
        // The loop should never exit without an error
        Ok(_) => error!("Loop should never break without error"),
        Err(e) => error!("{e}"),
    }

    // Exit with error code if we reach here
    exit(1);
}
