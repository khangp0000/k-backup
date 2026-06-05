//! SQLite database backup source using the backup API.

use crate::config::SqliteSourceConfig;
use crate::error::ArchiveError;
use crate::pipeline::entry::{ArchiveEntry, ArchiveEntryKind};
use rusqlite::{Connection, OpenFlags};
use std::fs::File;

/// Pre-validates that the SQLite database exists and is readable.
pub fn validate(config: &SqliteSourceConfig) -> std::result::Result<(), ArchiveError> {
    Connection::open_with_flags(&config.src, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map(|_| ())
        .map_err(ArchiveError::from)
}

/// Creates an archive entry by backing up the SQLite database to a temp file.
pub fn create_entry(
    config: &SqliteSourceConfig,
    temp_dir: Option<&std::path::Path>,
) -> std::result::Result<ArchiveEntry, ArchiveError> {
    let tmp_named = match temp_dir {
        Some(dir) => tempfile::NamedTempFile::new_in(dir),
        None => tempfile::NamedTempFile::new(),
    }
    .map_err(ArchiveError::from)?;

    let temp_path = tmp_named.into_temp_path();

    {
        let src_conn = Connection::open_with_flags(&config.src, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut dst_conn = Connection::open(&temp_path)?;
        let backup = rusqlite::backup::Backup::new(&src_conn, &mut dst_conn)?;
        backup.run_to_completion(100, std::time::Duration::from_millis(10), None)?;
    }

    let file = File::open(&temp_path).map_err(ArchiveError::from)?;

    Ok(ArchiveEntry {
        dst: config.dst.clone(),
        kind: ArchiveEntryKind::TempFile(file, temp_path),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn validate_nonexistent_file_returns_error() {
        let config = SqliteSourceConfig {
            src: "/nonexistent/db.sqlite3".into(),
            dst: "db.sqlite3".into(),
            required: true,
        };
        assert!(validate(&config).is_err());
    }

    #[test]
    fn create_entry_produces_valid_sqlite() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");

        // Create a real SQLite database
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE t(x INTEGER); INSERT INTO t VALUES(42);")
            .unwrap();
        drop(conn);

        let config = SqliteSourceConfig {
            src: db_path,
            dst: "backup.db".into(),
            required: true,
        };
        let entry = create_entry(&config, Some(tmp.path())).unwrap();
        assert_eq!(entry.dst, PathBuf::from("backup.db"));
        match entry.kind {
            ArchiveEntryKind::TempFile(_, _) => {} // good
            _ => panic!("expected TempFile kind"),
        }
    }
}
