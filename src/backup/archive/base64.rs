use crate::backup::archive::{ArchiveEntry, ArchiveEntryIterable};
use crate::backup::arcvec::ArcVec;
use crate::backup::result_error::result::Result;

use bon::Builder;
use dyn_iter::{DynIter, IntoDynIterator};
use getset::Getters;
use serde::{Deserialize, Serialize};
use serde_with::base64::Base64;
use serde_with::As;
use validator::Validate;

use std::io::Cursor;
use std::path::PathBuf;

/// Base64-encoded content source for archive entries
///
/// This source type allows creating archive entries from base64-encoded content,
/// which is particularly useful for testing, configuration data, and small
/// in-memory content that needs to be included in backups.
/// 
/// Content is automatically decoded from base64 during serialization/deserialization.
#[derive(Clone, Debug, Serialize, Deserialize, Validate, Builder, PartialEq, Eq, Getters)]
#[serde(deny_unknown_fields)]
#[getset(get = "pub")]
pub struct Base64Source {
    /// Base64-encoded content
    #[serde(with = "As::<Base64>")]
    #[builder(into)]
    content: ArcVec<u8>,
    /// Destination path within the archive
    #[builder(into)]
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
    fn test_base64_source_iterator() {
        let original_content = "Hello, World!";

        let source = Base64Source::builder()
            .content(original_content)
            .dst(PathBuf::from("hello.txt"))
            .build();

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
}
