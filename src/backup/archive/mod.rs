pub mod base64;
pub mod sqlite;
pub mod walkdir_globset;

use crate::backup::archive::base64::Base64Source;
use crate::backup::archive::sqlite::SqliteDBSource;
use crate::backup::archive::walkdir_globset::WalkdirAndGlobsetSource;
use crate::backup::result_error::result::Result;
use crate::backup::result_error::AddDebugObjectAndFnName;
use derive_more::From;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;

/// Trait combining Read, Send, and Debug for readable sources
pub trait ReadableSource: Read + Send + std::fmt::Debug {}

/// Blanket implementation for all types that implement Read + Send + Debug
impl<T> ReadableSource for T where T: Read + Send + std::fmt::Debug {}

/// Trait combining AsRef<Path>, Send, and Debug for file system paths
pub trait PathSource: AsRef<Path> + Send + std::fmt::Debug {}

/// Blanket implementation for all types that implement AsRef<Path> + Send + Debug
impl<T> PathSource for T where T: AsRef<Path> + Send + std::fmt::Debug {}

/// Trait combining AsRef<Path>, Send, and Debug for archive paths
pub trait ArchivePath: AsRef<Path> + Send + std::fmt::Debug {}

/// Blanket implementation for all types that implement AsRef<Path> + Send + Debug
impl<T> ArchivePath for T where T: AsRef<Path> + Send + std::fmt::Debug {}

/// Configuration for different types of backup sources
///
/// This enum represents the different ways files can be selected for backup:
/// - Sqlite: Proper backup of SQLite database files using SQLite's backup API
/// - Glob: File/directory selection using glob patterns and directory walking
/// - Base64: In-memory content encoded as base64 (useful for testing)
#[derive(Clone, From, Serialize, Deserialize, Debug)]
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
    fn new_path<
        A: AsRef<Path> + Send + std::fmt::Debug + 'static,
        B: AsRef<Path> + Send + std::fmt::Debug + 'static,
    >(
        src: A,
        dst: B,
    ) -> ArchiveEntry {
        Self {
            src: ArchiveSource::Path(Box::new(src)),
            dst: Box::new(dst),
        }
    }

    /// Creates a new archive entry with reader source
    pub fn new_reader<B: AsRef<Path> + Send + std::fmt::Debug + 'static>(
        src: Box<dyn ReadableSource>,
        dst: B,
    ) -> ArchiveEntry {
        Self {
            src: ArchiveSource::Reader(src),
            dst: Box::new(dst),
        }
    }

    /// Creates an archive entry from source and destination paths
    pub fn new<
        A: AsRef<Path> + Send + std::fmt::Debug + 'static,
        B: AsRef<Path> + Send + std::fmt::Debug + 'static,
    >(
        src: A,
        dst: B,
    ) -> ArchiveEntry {
        Self::new_path(src, dst)
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
    fn archive_entry_iterator(
        &self,
    ) -> Result<Box<dyn Iterator<Item = Result<ArchiveEntry>> + Send>>;
}

impl ArchiveEntryIterable for ArchiveEntryConfig {
    fn archive_entry_iterator(
        &self,
    ) -> Result<Box<dyn Iterator<Item = Result<ArchiveEntry>> + Send>> {
        match self {
            ArchiveEntryConfig::Sqlite(c) => c.archive_entry_iterator(),
            ArchiveEntryConfig::Glob(c) => c.archive_entry_iterator(),
            ArchiveEntryConfig::Base64(c) => c.archive_entry_iterator(),
        }
        .add_debug_object_and_fn_name(self.clone(), "archive_entry_iterator")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_archive_entry_creation() {
        let src = PathBuf::from("/source/file.txt");
        let dst = PathBuf::from("backup/file.txt");

        let entry = ArchiveEntry::new(src.clone(), dst.clone());
        if let ArchiveSource::Path(path) = &entry.src {
            assert_eq!(path.as_ref().as_ref(), src.as_path());
        } else {
            panic!("Expected path source");
        }
        assert_eq!(entry.dst.as_ref().as_ref(), dst.as_path());
    }

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

    #[test]
    fn test_archive_entry_debug() {
        let entry = ArchiveEntry::new(PathBuf::from("/src"), PathBuf::from("dst"));
        let debug_str = format!("{:?}", entry);
        assert_eq!(
            debug_str,
            "ArchiveEntry { src: Path(\"/src\"), dst: \"dst\" }"
        );
    }
}
