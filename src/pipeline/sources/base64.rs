//! Base64-encoded content source.

use crate::config::Base64SourceConfig;
use crate::pipeline::entry::{ArchiveEntry, ArchiveEntryKind};
use std::sync::Arc;

/// Creates an archive entry from pre-decoded base64 content.
pub fn create_entry(config: &Base64SourceConfig) -> ArchiveEntry {
    ArchiveEntry {
        dst: config.dst.clone(),
        kind: ArchiveEntryKind::Memory(Arc::clone(config.content.arc())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Base64Bytes;
    use std::path::PathBuf;

    #[test]
    fn create_entry_produces_memory_kind() {
        let config = Base64SourceConfig {
            content: Base64Bytes::new(b"Hello".to_vec()),
            dst: "msg.txt".into(),
        };
        let entry = create_entry(&config);
        assert_eq!(entry.dst, PathBuf::from("msg.txt"));
        match entry.kind {
            ArchiveEntryKind::Memory(data) => assert_eq!(&*data, b"Hello"),
            _ => panic!("expected Memory kind"),
        }
    }

    #[test]
    fn invalid_base64_rejected_at_deserialization() {
        let yaml = r#"
content: "!!!invalid!!!"
dst: bad.txt
"#;
        let result: Result<Base64SourceConfig, _> = serde_saphyr::from_str(yaml);
        assert!(result.is_err());
    }
}
