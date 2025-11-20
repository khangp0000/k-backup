use crate::backup::archive::{ArchiveEntry, ArchiveEntryIterable};
use crate::backup::result_error::result::Result;
use rusqlite::{Connection, OpenFlags};
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
#[serde(deny_unknown_fields)]
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
        tracing::info!("Starting SQLite backup for database: {:?}", self.src);
        
        // Open database in read-only mode with no mutex (safe for backup)
        let conn = Connection::open_with_flags(
            self.src.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;

        // Create temporary file for the backup copy
        let temp_file_path = Builder::new().keep(true).tempfile()?.path().to_path_buf();
        tracing::info!("Creating temporary backup file: {:?}", temp_file_path);
        
        // Use SQLite's backup API to create consistent snapshot
        tracing::debug!("Creating SQLite backup from {:?} to {:?}", self.src, temp_file_path);
        conn.backup(rusqlite::MAIN_DB, &temp_file_path, None)?;
        
        let file_size = std::fs::metadata(&temp_file_path)
            .map(|m| m.len())
            .unwrap_or(0);
        tracing::info!("SQLite backup completed successfully ({} bytes)", file_size);
        
        // Return entry that will delete temp file after backup
        tracing::info!("SQLite backup entry created: {:?} -> {:?}", temp_file_path, self.dst);
        Ok(Box::new(std::iter::once(Ok(ArchiveEntry::delete_src(
            temp_file_path,
            self.dst.clone(),
        )))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_test_database(path: &Path) -> Result<()> {
        let conn = Connection::open(path)?;
        conn.execute(
            "CREATE TABLE test_table (id INTEGER PRIMARY KEY, name TEXT)",
            [],
        )?;
        conn.execute(
            "INSERT INTO test_table (name) VALUES ('test_data')",
            [],
        )?;
        Ok(())
    }

    #[test]
    fn test_sqlite_db_source_creation() {
        let src = PathBuf::from("/path/to/database.db");
        let dst = PathBuf::from("backup/database.db");
        
        let source = SqliteDBSource {
            src: src.clone().into(),
            dst: dst.clone().into(),
        };
        
        assert_eq!(source.src.as_ref(), src.as_path());
        assert_eq!(source.dst.as_ref(), dst.as_path());
    }

    #[test]
    fn test_sqlite_db_source_serialization() {
        let source = SqliteDBSource {
            src: PathBuf::from("/path/to/database.db").into(),
            dst: PathBuf::from("backup/database.db").into(),
        };
        
        let serialized = serde_json::to_string(&source).unwrap();
        let deserialized: SqliteDBSource = serde_json::from_str(&serialized).unwrap();
        
        assert_eq!(source.src, deserialized.src);
        assert_eq!(source.dst, deserialized.dst);
    }

    #[test]
    fn test_sqlite_db_source_debug() {
        let source = SqliteDBSource {
            src: PathBuf::from("/path/to/database.db").into(),
            dst: PathBuf::from("backup/database.db").into(),
        };
        
        let debug_str = format!("{:?}", source);
        assert!(debug_str.contains("SqliteDBSource"));
        assert!(debug_str.contains("database.db"));
    }

    #[test]
    fn test_archive_entry_iterator_with_valid_database() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        
        // Create a test database
        create_test_database(&db_path).unwrap();
        
        let source = SqliteDBSource {
            src: db_path.into(),
            dst: PathBuf::from("backup/test.db").into(),
        };
        
        let iterator = source.archive_entry_iterator().unwrap();
        let entries: Vec<_> = iterator.collect();
        
        assert_eq!(entries.len(), 1);
        
        let entry = entries[0].as_ref().unwrap();
        assert!(entry.delete_src); // Should delete temp file after backup
        assert_eq!(entry.dst.as_ref(), Path::new("backup/test.db"));
        
        // Verify the temp file exists and contains data
        assert!(entry.src.exists());
        
        // Verify we can open the backup database
        let backup_conn = Connection::open(&entry.src).unwrap();
        let mut stmt = backup_conn.prepare("SELECT COUNT(*) FROM test_table").unwrap();
        let count: i64 = stmt.query_row([], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_archive_entry_iterator_with_nonexistent_database() {
        let source = SqliteDBSource {
            src: PathBuf::from("/nonexistent/database.db").into(),
            dst: PathBuf::from("backup/database.db").into(),
        };
        
        let result = source.archive_entry_iterator();
        assert!(result.is_err());
    }

    #[test]
    fn test_archive_entry_iterator_with_invalid_database() {
        let temp_dir = TempDir::new().unwrap();
        let invalid_db_path = temp_dir.path().join("invalid.db");
        
        // Create a file that's not a valid SQLite database
        std::fs::write(&invalid_db_path, "not a database").unwrap();
        
        let source = SqliteDBSource {
            src: invalid_db_path.into(),
            dst: PathBuf::from("backup/invalid.db").into(),
        };
        
        let result = source.archive_entry_iterator();
        assert!(result.is_err());
    }

    #[test]
    fn test_sqlite_backup_consistency() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        
        // Create and populate test database
        create_test_database(&db_path).unwrap();
        
        // Add more data to test consistency
        let conn = Connection::open(&db_path).unwrap();
        for i in 2..=10 {
            conn.execute(
                "INSERT INTO test_table (name) VALUES (?)",
                [format!("test_data_{}", i)],
            ).unwrap();
        }
        drop(conn);
        
        let source = SqliteDBSource {
            src: db_path.into(),
            dst: PathBuf::from("backup/test.db").into(),
        };
        
        let iterator = source.archive_entry_iterator().unwrap();
        let entries: Vec<_> = iterator.collect();
        let entry = entries[0].as_ref().unwrap();
        
        // Verify backup contains all data
        let backup_conn = Connection::open(&entry.src).unwrap();
        let mut stmt = backup_conn.prepare("SELECT COUNT(*) FROM test_table").unwrap();
        let count: i64 = stmt.query_row([], |row| row.get(0)).unwrap();
        assert_eq!(count, 10);
    }
}