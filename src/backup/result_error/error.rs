use crate::backup::result_error::{AddFunctionName, AddMsg};
use std::borrow::Cow;
use std::fmt::Debug;
use std::sync::mpsc::SendError;
use thiserror::Error;
use thiserror_ext;

#[derive(Error, Debug, thiserror_ext::Box, thiserror_ext::Construct)]
#[thiserror_ext(
    newtype(name = Error),
)]
pub enum ErrorInternal {
    #[error(transparent)]
    Io(#[from] std::io::Error),
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
    #[error("{}() failed:\n{}", fn_name, indent::indent_all_with("  ", error.to_string()))]
    WithFnName {
        fn_name: Cow<'static, str>,
        error: Error,
    },
    #[error("{}", itertools::join(.0, "\n\n"))]
    LotsOfError(Vec<Error>),
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

impl Error {
    pub fn into_error_iter(self) -> Box<dyn Iterator<Item = Error>> {
        match self.into_inner() {
            ErrorInternal::LotsOfError(v) => Box::new(v.into_iter()),
            e => Box::new(std::iter::once(e.into())),
        }
    }

    pub fn chain(self, other: Error) -> Error {
        let error_vec = match self.into_inner() {
            ErrorInternal::LotsOfError(mut v) => {
                v.extend(other.into_error_iter());
                v
            }
            e => vec![e.into(), other],
        };
        Error::lots_of_error(error_vec)
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
