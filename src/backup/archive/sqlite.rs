use crate::backup::archive::{ArchiveEntry, ArchiveEntryIterable};
use crate::backup::result_error::result::Result;
use rusqlite::{Connection, DatabaseName, OpenFlags};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tempfile::Builder;

/// Configuration for backing up SQLite database files
/// 
/// Uses SQLite's built-in backup API to create consistent snapshots
/// even while the database is being actively used by other processes.
/// This is much safer than just copying the database file.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SqliteDBSource {
    /// Path to the source SQLite database file
    src: Arc<Path>,
    /// Destination path within the backup archive
    dst: Arc<Path>,
}

impl ArchiveEntryIterable for SqliteDBSource {
    /// Creates a temporary backup of the SQLite database
    /// 
    /// Process:
    /// 1. Opens the source database in read-only mode
    /// 2. Creates a temporary file for the backup
    /// 3. Uses SQLite's backup API to copy the database
    /// 4. Returns an ArchiveEntry that will delete the temp file after backup
    /// 
    /// The temporary file is marked for deletion after being added to the archive.
    fn archive_entry_iterator(
        &self,
    ) -> Result<Box<dyn Iterator<Item = Result<ArchiveEntry>> + Send>> {
        // Open database in read-only mode with no mutex (safe for backup)
        let conn = Connection::open_with_flags(
            self.src.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;

        // Create temporary file for the backup copy
        let temp_file_path = Builder::new().keep(true).tempfile()?.path().to_path_buf();
        
        // Use SQLite's backup API to create consistent snapshot
        conn.backup(DatabaseName::Main, &temp_file_path, None)?;
        conn.backup(DatabaseName::Main, &temp_file_path, None)?;
        
        // Return entry that will delete temp file after backup
        Ok(Box::new(std::iter::once(Ok(ArchiveEntry::delete_src(
            temp_file_path,
            self.dst.clone(),
        )))))
    }
}
