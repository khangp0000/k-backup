use crate::backup::archive::{ArchiveEntry, ArchiveEntryIterable};
use crate::backup::result_error::result::Result;
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::path::PathBuf;

/// Base64-encoded content source for archive entries
///
/// This source type allows creating archive entries from base64-encoded content,
/// which is particularly useful for testing and small in-memory data.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Base64Source {
    /// Base64-encoded content
    content: String,
    /// Destination path within the archive
    dst: PathBuf,
}

impl Base64Source {
    pub fn new(content: String, dst: PathBuf) -> Self {
        Self { content, dst }
    }
}

impl ArchiveEntryIterable for Base64Source {
    fn archive_entry_iterator(
        &self,
    ) -> Result<Box<dyn Iterator<Item = Result<ArchiveEntry>> + Send>> {
        let decoded = general_purpose::STANDARD
            .decode(&self.content)
            .map_err(|e| {
                crate::backup::result_error::error::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    e,
                ))
            })?;

        let reader = Box::new(Cursor::new(decoded));
        let entry = ArchiveEntry::new_reader(reader, self.dst.clone());

        Ok(Box::new(std::iter::once(Ok(entry))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_base64_source_creation() {
        let content = general_purpose::STANDARD.encode("test content");
        let source = Base64Source::new(content.clone(), PathBuf::from("test.txt"));

        assert_eq!(source.content, content);
        assert_eq!(source.dst, PathBuf::from("test.txt"));
    }

    #[test]
    fn test_base64_source_iterator() {
        let original_content = "Hello, World!";
        let encoded_content = general_purpose::STANDARD.encode(original_content);

        let source = Base64Source::new(encoded_content, PathBuf::from("hello.txt"));

        let mut iterator = source.archive_entry_iterator().unwrap();
        let entry_result = iterator.next().unwrap();
        let mut entry = entry_result.unwrap();

        // Verify it's a reader source
        if let crate::backup::archive::ArchiveSource::Reader(ref mut reader) = entry.src {
            let mut content = String::new();
            reader.read_to_string(&mut content).unwrap();
            assert_eq!(content, original_content);
        } else {
            panic!("Expected reader source");
        }

        assert_eq!(entry.dst.as_ref().as_ref(), PathBuf::from("hello.txt"));
        assert!(iterator.next().is_none());
    }

    #[test]
    fn test_base64_source_invalid_base64() {
        let source = Base64Source::new("invalid base64!@#".to_string(), PathBuf::from("test.txt"));

        let result = source.archive_entry_iterator();
        assert!(result.is_err());
    }

    #[test]
    fn test_base64_source_serialization() {
        let source = Base64Source::new(
            general_purpose::STANDARD.encode("test"),
            PathBuf::from("test.txt"),
        );

        let serialized = serde_json::to_string(&source).unwrap();
        let deserialized: Base64Source = serde_json::from_str(&serialized).unwrap();

        assert_eq!(source.content, deserialized.content);
        assert_eq!(source.dst, deserialized.dst);
    }
}
