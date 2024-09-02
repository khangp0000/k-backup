use clap::Parser;
use k_backup::backup::backup_config::BackupConfig;
use k_backup::backup::result_error::error::Error;
use k_backup::backup::result_error::WithMsg;
use rayon::ThreadPoolBuilder;
use std::fs::File;
use std::path::PathBuf;
use std::process::exit;
use tracing::error;
use validator::Validate;

/// Simple(?) program to create backup and delete old backup
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Location of config file
    #[arg(short, long)]
    config: PathBuf,
}

fn main() {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let thread_pool = ThreadPoolBuilder::new().build().unwrap();

    let res = File::open(&args.config)
        .map_err(Error::from)
        .and_then(|f| {
            serde_yml::from_reader::<_, BackupConfig>(f)
                .map_err(Error::from)
                .with_msg(format!("Parse YAML config failed: {:?}", &args.config))
        })
        .and_then(|bc| {
            bc.validate()
                .map_err(Error::from)
                .map(|_| bc)
                .with_msg(format!("Config validation failed: {:?}", &args.config))
        })
        .and_then(|bc| bc.start_loop(thread_pool.into()));

    match res {
        Ok(_) => error!("Loop should never break without error"),
        Err(e) => error!("{e}"),
    }

    exit(1);
}
