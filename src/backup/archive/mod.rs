//! Archive entry sources and processing.
//!
//! Provides different sources for backup content:
//! - SQLite database backups using the backup API
//! - File/directory selection with glob patterns
//! - Base64-encoded content for testing

pub mod base64;
pub mod sqlite;
pub mod walkdir_globset;

use crate::backup::archive::base64::Base64Source;
use crate::backup::archive::sqlite::SqliteDBSource;
use crate::backup::archive::walkdir_globset::WalkdirAndGlobsetSource;
use crate::backup::result_error::result::Result;

use derive_more::From;
use dyn_iter::{DynIter, IntoDynIterator};
use serde::{Deserialize, Serialize};
use validator::{Validate, ValidationErrors};

use std::fmt::Debug;
use std::io::Read;
use std::path::Path;
use std::result;

/// Trait combining Read, Send, and Debug for readable sources
pub trait ReadableSource: Read + Send + Debug + 'static {}

/// Blanket implementation for all types that implement Read + Send + Debug
impl<T> ReadableSource for T where T: Read + Send + Debug + 'static {}

/// Trait combining `AsRef<Path>`, Send, and Debug for file system paths
pub trait PathSource: AsRef<Path> + Send + Debug + 'static {}

/// Blanket implementation for all types that implement `AsRef<Path>` + Send + Debug
impl<T> PathSource for T where T: AsRef<Path> + Send + Debug + 'static {}

/// Trait combining `AsRef<Path>`, Send, and Debug for archive paths
pub trait ArchivePath: AsRef<Path> + Send + Debug + 'static {}

/// Blanket implementation for all types that implement `AsRef<Path>` + Send + Debug
impl<T> ArchivePath for T where T: AsRef<Path> + Send + Debug + 'static {}

/// Configuration for different types of backup sources
///
/// This enum represents the different ways files can be selected for backup:
/// - Sqlite: Proper backup of SQLite database files using SQLite's backup API
/// - Glob: File/directory selection using glob patterns and directory walking
/// - Base64: In-memory content encoded as base64 (useful for testing)
#[derive(Clone, From, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum ArchiveEntryConfig {
    /// SQLite database backup configuration
    ///
    /// Uses SQLite's built-in backup API to create consistent database snapshots
    /// even while the database is being used by other processes.
    Sqlite(SqliteDBSource),

    /// File/directory glob pattern configuration
    ///
    /// Uses directory walking combined with glob pattern matching to select
    /// files and directories for backup. Supports complex include/exclude patterns.
    Glob(WalkdirAndGlobsetSource),

    /// Base64-encoded content configuration
    ///
    /// Creates archive entries from base64-encoded content.
    /// Primarily useful for testing and small in-memory content.
    Base64(Base64Source),
}

impl Validate for ArchiveEntryConfig {
    fn validate(&self) -> result::Result<(), ValidationErrors> {
        match self {
            ArchiveEntryConfig::Sqlite(i) => i.validate(),
            ArchiveEntryConfig::Glob(i) => i.validate(),
            ArchiveEntryConfig::Base64(i) => i.validate(),
        }
    }
}

/// Source type for archive entries
#[derive(Debug)]
pub enum ArchiveSource {
    /// File/directory path on the filesystem
    Path(Box<dyn PathSource>),
    /// Readable stream source
    Reader(Box<dyn ReadableSource>),
}

/// Represents a single file or directory to be included in a backup archive
///
/// Contains the source (path or reader) and destination path within the archive.
/// Temporary files are automatically cleaned up when dropped.
#[derive(Debug)]
pub struct ArchiveEntry {
    /// Source for the archive entry (path or readable stream)
    pub src: ArchiveSource,

    /// Destination path within the backup archive
    ///
    /// This determines the internal structure of the backup archive.
    /// Can be different from the source path to organize backups logically.
    pub dst: Box<dyn ArchivePath>,
}

impl ArchiveEntry {
    /// Creates a new archive entry with path source
    fn new_path<A: PathSource, B: ArchivePath>(src: A, dst: B) -> ArchiveEntry {
        Self {
            src: ArchiveSource::Path(Box::new(src)),
            dst: Box::new(dst),
        }
    }

    /// Creates a new archive entry with reader source
    pub fn new_reader<A: ReadableSource, B: ArchivePath>(src: A, dst: B) -> ArchiveEntry {
        Self {
            src: ArchiveSource::Reader(Box::new(src)),
            dst: Box::new(dst),
        }
    }
}

/// Trait for generating archive entries from configuration
///
/// Different backup source types (SQLite, glob patterns) implement this trait
/// to provide a unified interface for generating lists of files to backup.
pub trait ArchiveEntryIterable {
    /// Returns an iterator of archive entries to be included in the backup
    ///
    /// Each implementation scans its configured sources and generates
    /// ArchiveEntry objects representing files/directories to backup.
    ///
    /// The iterator yields Results to handle errors during source scanning
    /// (e.g., permission denied, missing files, etc.)
    fn archive_entry_iterator<'a>(&self) -> Result<DynIter<'a, Result<ArchiveEntry>>>;
}

impl ArchiveEntryIterable for ArchiveEntryConfig {
    fn archive_entry_iterator<'a>(&self) -> Result<DynIter<'a, Result<ArchiveEntry>>> {
        match self {
            ArchiveEntryConfig::Sqlite(c) => c.archive_entry_iterator(),
            ArchiveEntryConfig::Glob(c) => c.archive_entry_iterator(),
            ArchiveEntryConfig::Base64(c) => c.archive_entry_iterator(),
        }
        .or_else(|e| Ok(std::iter::once(Err(e)).into_dyn_iter()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_archive_entry_new_reader() {
        use std::io::Cursor;
        let data = b"test data";
        let reader = Box::new(Cursor::new(data.to_vec()));
        let dst = PathBuf::from("backup/file.txt");

        let entry = ArchiveEntry::new_reader(reader, dst.clone());
        matches!(entry.src, ArchiveSource::Reader(_));
        assert_eq!(entry.dst.as_ref().as_ref(), dst.as_path());
    }
}
