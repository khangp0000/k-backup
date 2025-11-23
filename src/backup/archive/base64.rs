use crate::backup::archive::{ArchiveEntry, ArchiveEntryIterable};
use crate::backup::result_error::result::Result;
use derive_ctor::ctor;
use derive_more::{Deref, DerefMut, From};
use dyn_iter::{DynIter, IntoDynIterator};
use serde::{Deserialize, Serialize};
use serde_with::base64::Base64;
use serde_with::As;
use std::io::Cursor;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use validator::{Validate, ValidateLength};

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

#[derive(
    From,
    Clone,
    Debug,
    Serialize,
    Deserialize,
    Ord,
    PartialOrd,
    Eq,
    PartialEq,
    ctor,
    Validate,
    Deref,
    DerefMut,
)]
#[ctor(pub new)]
#[serde(transparent)]
pub struct ArcVec<T> {
    #[ctor(into)]
    inner: Arc<Vec<T>>,
}

impl<T> ValidateLength<usize> for ArcVec<T> {
    fn length(&self) -> Option<usize> {
        Some(self.inner.len())
    }
}

impl<T> AsRef<[T]> for ArcVec<T> {
    fn as_ref(&self) -> &[T] {
        self.inner.deref().as_ref()
    }
}

macro_rules! impl_into_arcvec {
    ($ty:ty) => {
        impl<T> From<$ty> for ArcVec<T>
        where
            $ty: Into<Vec<T>>,
        {
            fn from(val: $ty) -> ArcVec<T> {
                ArcVec::from(val.into())
            }
        }
    };
    (a $ty:ty) => {
        impl<'a, T> From<$ty> for ArcVec<T>
        where
            $ty: Into<Vec<T>>,
        {
            fn from(val: $ty) -> ArcVec<T> {
                ArcVec::from(val.into())
            }
        }
    };
}

impl_into_arcvec!(String);
impl_into_arcvec!(Box<str>);
impl_into_arcvec!(Box<[T]>);
impl_into_arcvec!(a &'a str);
impl_into_arcvec!(a &'a String);
impl_into_arcvec!(a &'a [T]);

impl<T> From<Vec<T>> for ArcVec<T> {
    fn from(value: Vec<T>) -> Self {
        Self::new(value)
    }
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
