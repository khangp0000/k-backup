//! Archive entry types passed from collector to tar writer.

use std::fs::File;
use std::path::PathBuf;

/// A single entry to be added to the backup archive.
pub struct ArchiveEntry {
    /// Destination path within the archive.
    pub dst: PathBuf,
    /// Content source.
    pub kind: ArchiveEntryKind,
}

/// The kind of content for an archive entry.
pub enum ArchiveEntryKind {
    /// A file opened by the collector. Tar gets size from metadata, streams content.
    File(File),
    /// In-memory content (e.g., base64 decoded). Size is data.len().
    Memory(Vec<u8>),
    /// A symlink to store in the archive (preserve mode). Target is the link destination.
    Symlink(PathBuf),
}
