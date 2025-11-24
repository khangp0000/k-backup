use crate::backup::archive::{ArchiveEntry, ArchiveEntryIterable};
use crate::backup::arcvec::ArcVec;
use crate::backup::result_error::result::Result;
use derive_ctor::ctor;
use dyn_iter::{DynIter, IntoDynIterator};
use serde::{Deserialize, Serialize};
use serde_with::base64::Base64;
use serde_with::As;
use std::io::Cursor;
use std::path::PathBuf;
use validator::Validate;

/// Base64-encoded content source for archive entries
///
/// This source type allows creating archive entries from base64-encoded content,
/// which is particularly useful for testing and small in-memory data.
#[derive(Clone, Debug, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
#[derive(ctor)]
#[ctor(pub new)]
pub struct Base64Source {
    /// Base64-encoded content
    #[serde(with = "As::<Base64>")]
    #[ctor(into)]
    content: ArcVec<u8>,
    /// Destination path within the archive
    #[ctor(into)]
    dst: PathBuf,
}

impl ArchiveEntryIterable for Base64Source {
    fn archive_entry_iterator<'a>(&self) -> Result<DynIter<'a, Result<ArchiveEntry>>> {
        let reader = Cursor::new(self.content.clone());
        let entry = ArchiveEntry::new_reader(reader, self.dst.clone());

        Ok(std::iter::once(Ok(entry)).into_dyn_iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_base64_source_creation() {
        let content = vec![1, 2, 3, 4, 5];
        let source = Base64Source::new(content.clone(), PathBuf::from("test.txt"));

        assert_eq!(source.content.as_ref(), content);
        assert_eq!(source.dst, PathBuf::from("test.txt"));
    }

    #[test]
    fn test_base64_source_iterator() {
        let original_content = "Hello, World!";

        let source = Base64Source::new(original_content, PathBuf::from("hello.txt"));

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
    fn test_base64_source_serialization() {
        let source = Base64Source::new("test", PathBuf::from("test.txt"));

        let serialized = serde_json::to_string(&source).unwrap();
        let deserialized: Base64Source = serde_json::from_str(&serialized).unwrap();
        println!("{}", serialized);
        assert_eq!(source.content, deserialized.content);
        assert_eq!(source.dst, deserialized.dst);
    }
}
