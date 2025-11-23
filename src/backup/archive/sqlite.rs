use crate::backup::archive::{ArchiveEntry, ArchiveEntryIterable};
use crate::backup::function_path;
use crate::backup::result_error::result::Result;
use crate::backup::result_error::AddFunctionName;
use crate::backup::validate::validate_sql_file;
use derive_ctor::ctor;
use dyn_iter::{DynIter, IntoDynIterator};
use function_name::named;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::path::PathBuf;
use tempfile::NamedTempFile;
use validator::Validate;

/// Configuration for backing up SQLite database files
///
/// Uses SQLite's built-in backup API to create consistent snapshots
/// even while the database is being actively used by other processes.
/// This is much safer than just copying the database file.
#[derive(Serialize, Deserialize, Debug, Clone, Validate)]
#[serde(deny_unknown_fields)]
#[derive(ctor)]
#[ctor(pub new)]
pub struct SqliteDBSource {
    /// Path to the source SQLite database file
    #[ctor(into)]
    #[validate(custom(function = validate_sql_file))]
    src: PathBuf,
    /// Destination path within the backup archive
    #[ctor(into)]
    dst: PathBuf,
}

impl SqliteDBSource {
    fn create_archive_entry(&self) -> Result<ArchiveEntry> {
        tracing::info!("Starting SQLite backup for database: {:?}", self.src);

        // Open database in read-only mode with no mutex (safe for backup)
        let conn = Connection::open_with_flags(
            &self.src,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;

        // Create temporary file for the backup copy (will auto-delete when dropped)
        let temp_file = NamedTempFile::new()?;
        tracing::info!("Creating temporary backup file: {:?}", temp_file.path());

        // Use SQLite's backup API to create consistent snapshot
        tracing::debug!(
            "Creating SQLite backup from {:?} to {:?}",
            self.src,
            temp_file.path()
        );
        conn.backup(rusqlite::MAIN_DB, &temp_file, None)?;

        let file_size = std::fs::metadata(&temp_file).map(|m| m.len()).unwrap_or(0);
        tracing::info!("SQLite backup completed successfully ({} bytes)", file_size);

        // Return entry with temp file (will auto-cleanup when dropped)
        tracing::info!(
            "SQLite backup entry created: {:?} -> {:?}",
            temp_file.path(),
            self.dst
        );
        Ok(ArchiveEntry::new_path(temp_file, self.dst.clone()))
    }
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
    #[named]
    fn archive_entry_iterator<'a>(&self) -> Result<DynIter<'a, Result<ArchiveEntry>>> {
        Ok(
            std::iter::once(self.create_archive_entry().add_fn_name(function_path!()))
                .into_dyn_iter(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::path::Path;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_test_database(path: &Path) -> Result<()> {
        let conn = Connection::open(path)?;
        conn.execute(
            "CREATE TABLE test_table (id INTEGER PRIMARY KEY, name TEXT)",
            [],
        )?;
        conn.execute("INSERT INTO test_table (name) VALUES ('test_data')", [])?;
        Ok(())
    }

    #[test]
    fn test_sqlite_db_source_creation() {
        let src = PathBuf::from("/path/to/database.db");
        let dst = PathBuf::from("backup/database.db");

        let source = SqliteDBSource::new(src.clone(), dst.clone());

        assert_eq!(source.src, src);
        assert_eq!(source.dst, dst);
    }

    #[test]
    fn test_sqlite_db_source_serialization() {
        let source = SqliteDBSource::new(
            PathBuf::from("/path/to/database.db"),
            PathBuf::from("backup/database.db"),
        );

        let serialized = serde_json::to_string(&source).unwrap();
        let deserialized: SqliteDBSource = serde_json::from_str(&serialized).unwrap();

        assert_eq!(source.src, deserialized.src);
        assert_eq!(source.dst, deserialized.dst);
    }

    #[test]
    fn test_sqlite_db_source_debug() {
        let source = SqliteDBSource::new(
            PathBuf::from("/path/to/database.db"),
            PathBuf::from("backup/database.db"),
        );

        let debug_str = format!("{:?}", source);
        assert_eq!(
            debug_str,
            "SqliteDBSource { src: \"/path/to/database.db\", dst: \"backup/database.db\" }"
        );
    }

    #[test]
    fn test_archive_entry_iterator_with_valid_database() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");

        // Create a test database
        create_test_database(&db_path).unwrap();

        let source = SqliteDBSource::new(db_path, PathBuf::from("backup/test.db"));

        let iterator = source.archive_entry_iterator().unwrap();
        let entries: Vec<_> = iterator.collect();

        assert_eq!(entries.len(), 1);

        let entry = entries[0].as_ref().unwrap();
        // Temp file will be automatically cleaned up when dropped
        assert_eq!(entry.dst.as_ref().as_ref(), Path::new("backup/test.db"));

        // Verify the temp file exists and contains data
        if let crate::backup::archive::ArchiveSource::Path(path) = &entry.src {
            assert!(path.as_ref().as_ref().exists());

            // Verify we can open the backup database
            let backup_conn = Connection::open(path.as_ref().as_ref()).unwrap();
            let mut stmt = backup_conn
                .prepare("SELECT COUNT(*) FROM test_table")
                .unwrap();
            let count: i64 = stmt.query_row([], |row| row.get(0)).unwrap();
            assert_eq!(count, 1);
        } else {
            panic!("Expected path source");
        }
    }

    #[test]
    fn test_archive_entry_iterator_with_nonexistent_database() {
        let source = SqliteDBSource::new(
            PathBuf::from("/nonexistent/database.db"),
            PathBuf::from("backup/database.db"),
        );

        let result = source.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_archive_entry_iterator_with_invalid_database() {
        let temp_dir = TempDir::new().unwrap();
        let invalid_db_path = temp_dir.path().join("invalid.db");

        // Create a file that's not a valid SQLite database
        std::fs::write(&invalid_db_path, "not a database").unwrap();

        let source = SqliteDBSource::new(invalid_db_path, PathBuf::from("backup/invalid.db"));

        let result = source.archive_entry_iterator().unwrap().next().unwrap();
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
            )
            .unwrap();
        }
        drop(conn);

        let source = SqliteDBSource::new(db_path, PathBuf::from("backup/test.db"));

        let iterator = source.archive_entry_iterator().unwrap();
        let entries: Vec<_> = iterator.collect();
        let entry = entries[0].as_ref().unwrap();

        // Verify backup contains all data
        if let crate::backup::archive::ArchiveSource::Path(path) = &entry.src {
            let backup_conn = Connection::open(path.as_ref().as_ref()).unwrap();
            let mut stmt = backup_conn
                .prepare("SELECT COUNT(*) FROM test_table")
                .unwrap();
            let count: i64 = stmt.query_row([], |row| row.get(0)).unwrap();
            assert_eq!(count, 10);
        } else {
            panic!("Expected path source");
        }
    }

    #[test]
    fn test_temp_file_cleanup_after_drop() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");

        // Create a test database
        create_test_database(&db_path).unwrap();

        let source = SqliteDBSource::new(db_path, PathBuf::from("backup/test.db"));

        let temp_file_path = {
            let iterator = source.archive_entry_iterator().unwrap();
            let entries: Vec<_> = iterator.collect();
            let entry = entries[0].as_ref().unwrap();

            if let crate::backup::archive::ArchiveSource::Path(path) = &entry.src {
                let temp_path = path.as_ref().as_ref().to_path_buf();
                assert!(temp_path.exists());
                temp_path
            } else {
                panic!("Expected path source");
            }
        }; // ArchiveEntry dropped here

        // Verify temp file is deleted after drop
        assert!(!temp_file_path.exists());
    }
}
