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

#[derive(Clone, From, Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ArchiveEntryConfig {
    Sqlite(SqliteDBSource),
    Glob(WalkdirAndGlobsetSource),
}

#[derive(Debug)]
pub struct ArchiveEntry {
    pub src: Arc<Path>,
    pub dst: Arc<Path>,
    pub delete_src: bool,
}

impl ArchiveEntry {
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

    fn keep_src<A: Into<Arc<Path>>, B: Into<Arc<Path>>>(src: A, dst: B) -> ArchiveEntry {
        Self::new(src, dst, false)
    }

    fn delete_src<A: Into<Arc<Path>>, B: Into<Arc<Path>>>(src: A, dst: B) -> ArchiveEntry {
        Self::new(src, dst, true)
    }
}

pub trait ArchiveEntryIterable {
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
