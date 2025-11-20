pub mod sqlite;
pub mod walkdir_globset;

use crate::backup::archive::sqlite::SqliteDBSource;
use crate::backup::archive::walkdir_globset::WalkdirAndGlobsetSource;
use crate::backup::result_error::result::Result;
use crate::backup::result_error::WithDebugObjectAndFnName;
use derive_more::From;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

/// Configuration for different types of backup sources
/// 
/// This enum represents the different ways files can be selected for backup:
/// - Sqlite: Proper backup of SQLite database files using SQLite's backup API
/// - Glob: File/directory selection using glob patterns and directory walking
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
}

/// Represents a single file or directory to be included in a backup archive
/// 
/// Contains the source path, destination path within the archive,
/// and whether the source should be deleted after backup (for temporary files).
#[derive(Debug)]
pub struct ArchiveEntry {
    /// Source file/directory path on the filesystem
    pub src: Arc<Path>,
    
    /// Destination path within the backup archive
    /// 
    /// This determines the internal structure of the backup archive.
    /// Can be different from the source path to organize backups logically.
    pub dst: Arc<Path>,
    
    /// Whether to delete the source file after successful backup
    /// 
    /// Used for temporary files (like SQLite backup copies) that should
    /// be cleaned up after being added to the archive.
    pub delete_src: bool,
}

impl ArchiveEntry {
    /// Creates a new archive entry with specified deletion behavior
    fn new<A: Into<Arc<Path>>, B: Into<Arc<Path>>>(
        src: A,
        dst: B,
        delete_src: bool,
    ) -> ArchiveEntry {
        Self {
            src: src.into(),
            dst: dst.into(),
            delete_src,
        }
    }

    /// Creates an archive entry that preserves the source file after backup
    /// 
    /// Used for regular files that should remain on the filesystem
    /// after being backed up.
    pub fn keep_src<A: Into<Arc<Path>>, B: Into<Arc<Path>>>(src: A, dst: B) -> ArchiveEntry {
        Self::new(src, dst, false)
    }

    /// Creates an archive entry that deletes the source file after backup
    /// 
    /// Used for temporary files (like SQLite backup copies) that should
    /// be cleaned up after being successfully added to the archive.
    pub fn delete_src<A: Into<Arc<Path>>, B: Into<Arc<Path>>>(src: A, dst: B) -> ArchiveEntry {
        Self::new(src, dst, true)
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
        }
        .with_debug_object_and_fn_name(self.clone(), "archive_entry_iterator")
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
        
        let entry = ArchiveEntry::keep_src(src.clone(), dst.clone());
        assert_eq!(entry.src.as_ref(), src.as_path());
        assert_eq!(entry.dst.as_ref(), dst.as_path());
        assert!(!entry.delete_src);
        
        let entry = ArchiveEntry::delete_src(src.clone(), dst.clone());
        assert_eq!(entry.src.as_ref(), src.as_path());
        assert_eq!(entry.dst.as_ref(), dst.as_path());
        assert!(entry.delete_src);
    }

    #[test]
    fn test_archive_entry_new() {
        let src = PathBuf::from("/source/file.txt");
        let dst = PathBuf::from("backup/file.txt");
        
        let entry = ArchiveEntry::new(src.clone(), dst.clone(), true);
        assert_eq!(entry.src.as_ref(), src.as_path());
        assert_eq!(entry.dst.as_ref(), dst.as_path());
        assert!(entry.delete_src);
    }

    #[test]
    fn test_archive_entry_debug() {
        let entry = ArchiveEntry::keep_src(PathBuf::from("/src"), PathBuf::from("dst"));
        let debug_str = format!("{:?}", entry);
        assert!(debug_str.contains("src"));
        assert!(debug_str.contains("dst"));
        assert!(debug_str.contains("delete_src"));
    }
}