//! Core backup functionality and configuration.
//!
//! This module contains all the components needed for creating automated backups:
//! - Archive sources (SQLite, files, base64 content)
//! - Compression and encryption pipelines
//! - Retention policies and cleanup
//! - Configuration management
//! - Error handling utilities

pub mod archive;
pub mod arcvec;
pub mod backup_config;
pub mod compress;
pub mod encrypt;
pub mod file_ext;
pub mod finish;
pub mod notifications;
pub mod redacted;
pub mod result_error;
pub mod retention;
pub mod tar;
pub mod validate;

macro_rules! function_path {
    () => {
        concat!(
            module_path!(),
            "::",
            function_name!(),
            " ",
            file!(),
            ":",
            line!()
        )
    };
}

pub(crate) use function_path;
