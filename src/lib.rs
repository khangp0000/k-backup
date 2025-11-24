//! # k-backup
//!
//! An automated backup tool with encryption, compression, and retention management.
//!
//! ## Features
//!
//! - **Scheduled Backups**: Cron-based automation
//! - **Multiple Sources**: SQLite databases and file/directory patterns
//! - **Compression**: XZ (LZMA) with parallel processing
//! - **Encryption**: Age encryption with passphrase support
//! - **Retention Management**: Configurable policies with grandfather-father-son rotation
//! - **Parallel Processing**: Multi-threaded operations for performance
//!
//! ## Quick Start
//!
//! ```no_run
//! use k_backup::backup::backup_config::BackupConfig;
//! 
//! // Load configuration from YAML file
//! let config: BackupConfig = serde_yml::from_reader(std::fs::File::open("config.yml")?)?;
//! 
//! // Start the backup daemon
//! let thread_pool = rayon::ThreadPoolBuilder::new().build()?;
//! config.start_loop(std::sync::Arc::new(thread_pool))?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod backup;
