//! Validation functions for configuration values.
//!
//! Provides custom validation functions for file paths, directories,
//! cron expressions, and other configuration parameters.

use chrono::Utc;
use rusqlite::{Connection, OpenFlags};
use sanitize_filename::{is_sanitized, sanitize};
use validator::ValidationError;

use std::path::Path;
use std::result;

pub fn validate_valid_archive_base_name<S: AsRef<str>>(name: S) -> Result<(), ValidationError> {
    if !is_sanitized(name.as_ref()) {
        return Err(ValidationError::new("InvalidArchiveBaseName").with_message(
            format!(
                "Invalid file name, try sanitizing like {:?}",
                sanitize(name)
            )
            .into(),
        ));
    }

    Ok(())
}

pub fn validate_dir_exist<P: AsRef<Path>>(dir: P) -> Result<(), ValidationError> {
    let dir = dir.as_ref();
    if dir.exists() {
        if !dir.is_dir() {
            return Err(ValidationError::new("InvalidDirectory")
                .with_message(format!("{:?} is not a directory", dir).into()));
        }
    } else {
        return Err(ValidationError::new("InvalidDirectory")
            .with_message(format!("{:?} not found", dir).into()));
    }

    Ok(())
}

pub fn validate_dir_exist_or_created<P: AsRef<Path>>(dir: P) -> Result<(), ValidationError> {
    let dir = dir.as_ref();
    if dir.exists() {
        if !dir.is_dir() {
            return Err(ValidationError::new("InvalidDirectory")
                .with_message(format!("{:?} is not a directory", dir).into()));
        }
    } else {
        return std::fs::create_dir_all(dir).map_err(|e| {
            ValidationError::new("InvalidDirectory").with_message(
                format!("cannot create or access out_dir path {:?}: {}", dir, e).into(),
            )
        });
    }

    Ok(())
}

pub fn validate_writable_dir<P: AsRef<Path>>(dir: P) -> Result<(), ValidationError> {
    let dir = dir.as_ref();
    validate_dir_exist_or_created(dir)?;
    let md = std::fs::metadata(dir).map_err(|e| {
        ValidationError::new("InvalidDirectory")
            .with_message(format!("cannot access metadata for {:?}: {}", dir, e).into())
    })?;
    if md.permissions().readonly() {
        Err(ValidationError::new("InvalidDirectory")
            .with_message(format!("cannot write ti dir {:?}", dir).into()))
    } else {
        Ok(())
    }
}

pub fn validate_cron_str<S: AsRef<str>>(cron: S) -> Result<(), ValidationError> {
    let cron = cron.as_ref();
    if cron_parser::parse(cron, &Utc::now()).is_err() {
        return Err(ValidationError::new("InvalidCron")
            .with_message(format!("Invalid cron string: {cron:?}").into()));
    }

    Ok(())
}

pub fn validate_sql_file<P: AsRef<Path>>(path: P) -> result::Result<(), ValidationError> {
    let path = path.as_ref();
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map(|_| ())
    .map_err(|e| {
        ValidationError::new("InvalidSqlFile")
            .with_message(format!("cannot open sql file {:?}: {}", path, e).into())
    })
}
