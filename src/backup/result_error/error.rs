use crate::backup::archive::ArchiveEntryConfig;
use crate::backup::arcvec::ArcVec;
use crate::backup::result_error::{AddFunctionName, AddMsg};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::{Debug, Display};
use std::sync::mpsc::SendError;
use std::sync::Arc;
use thiserror::Error;
use thiserror_ext;

/// A collection of non-fatal errors grouped by archive entry config index.
///
/// Each entry config (identified by its index in the config list) may produce
/// multiple errors during archive creation. Errors for the same index are
/// merged via [`Error::chain_inplace`].
///
/// Display format shows each failing entry's config followed by its indented errors.
pub struct EntryErrors {
    pub configs: ArcVec<ArchiveEntryConfig>,
    pub errors: BTreeMap<usize, Error>,
}

impl EntryErrors {
    pub fn new(configs: ArcVec<ArchiveEntryConfig>) -> Self {
        Self {
            configs,
            errors: BTreeMap::new(),
        }
    }

    /// Inserts an error for a given config index, merging with any existing error.
    pub fn insert(&mut self, index: usize, error: Error) {
        match self.errors.entry(index) {
            std::collections::btree_map::Entry::Occupied(mut e) => {
                e.get_mut().chain_inplace(error);
            }
            std::collections::btree_map::Entry::Vacant(e) => {
                e.insert(error);
            }
        }
    }

    /// Merges another `EntryErrors` into this one.
    ///
    /// Returns `Err` if the two instances refer to different config lists (pointer inequality).
    pub fn merge(&mut self, other: EntryErrors) -> std::result::Result<(), &'static str> {
        if !Arc::ptr_eq(&self.configs, &other.configs) {
            return Err("Cannot merge EntryErrors with different configs");
        }
        for (index, error) in other.errors {
            self.insert(index, error);
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }
}

impl Display for EntryErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, (index, error)) in self.errors.iter().enumerate() {
            if i > 0 {
                write!(f, "\n\n")?;
            }
            let config = &self.configs[*index];
            write!(f, "[entry #{}] {:?}\n", index, config)?;
            write!(f, "{}", indent::indent_all_with("  ", error.to_string()))?;
        }
        Ok(())
    }
}

impl Debug for EntryErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

#[derive(Error, Debug, thiserror_ext::Box, thiserror_ext::Construct)]
#[thiserror_ext(
    newtype(name = Error),
)]
pub enum ErrorInternal {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Smtp(#[from] lettre::transport::smtp::Error),
    #[error(transparent)]
    Lettre(#[from] lettre::error::Error),
    #[error(transparent)]
    StripPrefixError(#[from] std::path::StripPrefixError),
    #[error(transparent)]
    Rusqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    LiblzmaStream(#[from] liblzma::stream::Error),
    #[error(transparent)]
    ValidationError(#[from] validator::ValidationErrors),
    #[error(transparent)]
    ThreadPoolBuildError(#[from] rayon::ThreadPoolBuildError),
    #[error(transparent)]
    SerdeYml(#[from] serde_yml::Error),
    #[error(transparent)]
    WalkDir(#[from] walkdir::Error),
    #[error("{0}")]
    ChannelSendError(Cow<'static, str>),
    #[error("{}:\n{}", msg, indent::indent_all_with("  ", error.to_string()))]
    WithMsg {
        msg: Cow<'static, str>,
        error: Error,
    },
    #[error("{} failed:\n{}", fn_name, indent::indent_all_with("  ", error.to_string()))]
    WithFnName {
        fn_name: Cow<'static, str>,
        error: Error,
    },
    #[error("{}", itertools::join(.0, "\n\n"))]
    #[construct(skip)]
    LotsOfError(Vec<Error>),
    #[error("{0}")]
    SmtpSendError(Cow<'static, str>),
    #[error("{0}")]
    ArchiveEntryErrors(EntryErrors),
}

impl AddFunctionName for Error {
    fn add_fn_name<S: Into<Cow<'static, str>>>(self, fn_name: S) -> Self {
        Error::with_fn_name(fn_name.into(), self)
    }
}

impl<S: Into<Cow<'static, str>>> AddMsg<S> for Error {
    fn add_msg(self, msg: S) -> Self {
        Self::with_msg(msg.into(), self)
    }
}

impl<D: Debug> From<SendError<D>> for Error {
    fn from(value: SendError<D>) -> Self {
        Self::channel_send_error(format!("Failed to send {:?}", value.0))
    }
}

impl From<Vec<Error>> for Error {
    fn from(errors: Vec<Error>) -> Self {
        if errors.is_empty() {
            panic!("Should not create lots of errors when error is empty")
        }
        Self::lots_of_error(errors)
    }
}

const MAX_ERRORS: usize = 10;

impl Error {
    /// Creates a `LotsOfError`, truncating to [`MAX_ERRORS`] entries.
    pub fn lots_of_error(mut errors: Vec<Error>) -> Self {
        errors.truncate(MAX_ERRORS);
        ErrorInternal::LotsOfError(errors).into()
    }

    pub fn into_error_iter(self) -> Box<dyn ExactSizeIterator<Item = Error>> {
        match self.into_inner() {
            ErrorInternal::LotsOfError(v) => Box::new(v.into_iter()),
            e => Box::new(std::iter::once(e.into())),
        }
    }

    pub fn chain(self, other: Error) -> Error {
        let error_vec = match self.into_inner() {
            ErrorInternal::LotsOfError(mut v) => {
                let remaining = MAX_ERRORS.saturating_sub(v.len());
                v.extend(other.into_error_iter().take(remaining));
                v
            }
            e => {
                let other_iter = other.into_error_iter();
                let take = (MAX_ERRORS - 1).min(other_iter.len());
                let mut v = Vec::with_capacity(1 + take);
                v.push(e.into());
                v.extend(other_iter.take(take));
                v
            }
        };
        Error::lots_of_error(error_vec)
    }

    /// Appends `other` into this error in place, converting to `LotsOfError` if needed.
    ///
    /// Unlike [`chain`](Self::chain) which consumes `self`, this mutates in place —
    /// useful when the error is behind a mutable reference (e.g., in a map entry).
    /// Truncates to [`MAX_ERRORS`] entries.
    pub fn chain_inplace(&mut self, other: Error) {
        if let ErrorInternal::LotsOfError(v) = self.0.inner_mut() {
            let remaining = MAX_ERRORS.saturating_sub(v.len());
            v.extend(other.into_error_iter().take(remaining));
        } else {
            let other_iter = other.into_error_iter();
            let error_vec = Vec::with_capacity(1 + other_iter.len());
            let new_error = Error::lots_of_error(error_vec);
            let old_error = std::mem::replace(self, new_error);
            if let ErrorInternal::LotsOfError(v) = self.0.inner_mut() {
                v.push(old_error);
                let remaining = MAX_ERRORS.saturating_sub(v.len());
                v.extend(other_iter.take(remaining));
            } else {
                unreachable!()
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn test_error_from_io_error() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let error = Error::from(io_error);

        match error.inner() {
            ErrorInternal::Io(_) => (),
            _ => panic!("Expected Io error"),
        }
    }

    #[test]
    fn test_error_with_msg() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let error = Error::from(io_error);
        let error_with_msg = error.add_msg("Custom message");

        match error_with_msg.inner() {
            ErrorInternal::WithMsg { msg, .. } => assert_eq!(msg, "Custom message"),
            _ => panic!("Expected WithMsg error"),
        }
    }

    #[test]
    fn test_error_from_send_error() {
        let (tx, _rx) = mpsc::channel();
        drop(_rx); // Close receiver to cause send error

        let send_result = tx.send("test");
        match send_result {
            Err(send_error) => {
                let error = Error::from(send_error);
                match error.inner() {
                    ErrorInternal::ChannelSendError(_) => (),
                    _ => panic!("Expected ChannelSendError"),
                }
            }
            Ok(_) => panic!("Expected send error"),
        }
    }

    #[test]
    fn test_error_from_vec() {
        let errors = vec![
            Error::from(std::io::Error::new(std::io::ErrorKind::NotFound, "error1")),
            Error::from(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "error2",
            )),
        ];

        let combined_error = Error::from(errors);
        match combined_error.inner() {
            ErrorInternal::LotsOfError(error_vec) => assert_eq!(error_vec.len(), 2),
            _ => panic!("Expected LotsOfError"),
        }
    }

    #[test]
    #[should_panic(expected = "Should not create lots of errors when error is empty")]
    fn test_error_from_empty_vec_panics() {
        let errors: Vec<Error> = vec![];
        let _error = Error::from(errors);
    }

    #[test]
    fn test_error_into_iter() {
        let error = Error::from(std::io::Error::new(std::io::ErrorKind::NotFound, "test"));
        let mut iter = error.into_error_iter();

        assert!(iter.next().is_some());
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_error_into_iter_lots_of_error() {
        let errors = vec![
            Error::from(std::io::Error::new(std::io::ErrorKind::NotFound, "error1")),
            Error::from(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "error2",
            )),
        ];
        let combined_error = Error::from(errors);
        let iter = combined_error.into_error_iter();

        assert_eq!(iter.count(), 2);
    }

    #[test]
    fn test_error_chain() {
        let error1 = Error::from(std::io::Error::new(std::io::ErrorKind::NotFound, "error1"));
        let error2 = Error::from(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "error2",
        ));

        let chained = error1.chain(error2);
        match chained.inner() {
            ErrorInternal::LotsOfError(errors) => assert_eq!(errors.len(), 2),
            _ => panic!("Expected LotsOfError"),
        }
    }

    #[test]
    fn test_error_display() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let error = Error::from(io_error);
        let error_str = error.to_string();
        assert_eq!(error_str, "file not found");
    }

    #[test]
    fn test_error_with_msg_display() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let error = Error::from(io_error);
        let error_with_msg = error.add_msg("Operation failed");
        let error_str = error_with_msg.to_string();
        assert_eq!(error_str, "Operation failed:\n  file not found");
    }
}
