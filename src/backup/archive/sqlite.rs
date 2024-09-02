use crate::backup::archive::{ArchiveEntry, ArchiveEntryIterable};
use crate::backup::result_error::result::Result;
use rusqlite::{Connection, DatabaseName, OpenFlags};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tempfile::Builder;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SqliteDBSource {
    src: Arc<Path>,
    dst: Arc<Path>,
}

impl ArchiveEntryIterable for SqliteDBSource {
    fn archive_entry_iterator(
        &self,
    ) -> Result<Box<dyn Iterator<Item = Result<ArchiveEntry>> + Send>> {
        let conn = Connection::open_with_flags(
            self.src.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;

        let temp_file_path = Builder::new().keep(true).tempfile()?.path().to_path_buf();
        conn.backup(DatabaseName::Main, &temp_file_path, None)?;
        conn.backup(DatabaseName::Main, &temp_file_path, None)?;
        Ok(Box::new(std::iter::once(Ok(ArchiveEntry::delete_src(
            temp_file_path,
            self.dst.clone(),
        )))))
    }
}
