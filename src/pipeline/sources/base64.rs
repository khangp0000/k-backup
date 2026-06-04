//! Base64-encoded content source.

use crate::config::Base64SourceConfig;
use crate::error::ArchiveError;
use crate::pipeline::entry::{ArchiveEntry, ArchiveEntryKind};
use base64::Engine;

/// Creates an archive entry from base64-decoded content.
pub fn create_entry(config: &Base64SourceConfig) -> std::result::Result<ArchiveEntry, ArchiveError> {
    let data = base64::engine::general_purpose::STANDARD
        .decode(&config.content)
        .map_err(ArchiveError::from)?;

    Ok(ArchiveEntry {
        dst: config.dst.clone(),
        kind: ArchiveEntryKind::Memory(data),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn create_entry_valid_base64() {
        let config = Base64SourceConfig {
            content: "SGVsbG8=".into(), // "Hello"
            dst: "msg.txt".into(),
        };
        let entry = create_entry(&config).unwrap();
        assert_eq!(entry.dst, PathBuf::from("msg.txt"));
        match entry.kind {
            ArchiveEntryKind::Memory(data) => assert_eq!(data, b"Hello"),
            _ => panic!("expected Memory kind"),
        }
    }

    #[test]
    fn create_entry_invalid_base64() {
        let config = Base64SourceConfig {
            content: "!!!invalid!!!".into(),
            dst: "bad.txt".into(),
        };
        assert!(create_entry(&config).is_err());
    }
}
