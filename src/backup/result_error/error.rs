use crate::backup::result_error::{WithDebugObjectAndFnName, WithMsg};
use itertools::Itertools;
use std::fmt::Debug;
use std::sync::mpsc::SendError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
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
    ChannelSendError(String),
    #[error("{}:\n{}", msg, indent::indent_all_with("  ", error.to_string()))]
    WithMsg { msg: String, error: Box<Error> },
    #[error("{:?} {} failed:\n{}", obj_debug, fn_name, indent::indent_all_with("  ", error.to_string()))]
    WithDebugObjAndFnName {
        error: Box<Error>,
        obj_debug: Box<dyn Debug + Send>,
        fn_name: String,
    },
    #[error("{}", itertools::join(.0, "\n\n"))]
    LotsOfError(Vec<Error>),
}

impl<S: Into<String>, O: Debug + Send + 'static> WithDebugObjectAndFnName<S, O> for Error {
    fn with_debug_object_and_fn_name(self, obj: O, fn_name: S) -> Self {
        Error::WithDebugObjAndFnName {
            error: Box::new(self),
            obj_debug: Box::new(obj),
            fn_name: fn_name.into(),
        }
    }
}

impl<S: Into<String>> WithMsg<S> for Error {
    fn with_msg(self, msg: S) -> Self {
        Self::WithMsg {
            msg: msg.into(),
            error: Box::new(self),
        }
    }
}

impl<D: Debug> From<SendError<D>> for Error {
    fn from(value: SendError<D>) -> Self {
        Self::ChannelSendError(format!("Failed to send {:?}", value.0))
    }
}

impl From<Vec<Error>> for Error {
    fn from(errors: Vec<Error>) -> Self {
        if errors.is_empty() {
            panic!("Should not create lots of errors when error is empty")
        }
        Self::LotsOfError(
            errors
                .into_iter()
                .map(|e| e.into_iter())
                .flatten()
                .collect_vec(),
        )
    }
}

impl Error {
    pub fn into_iter(self) -> Box<dyn Iterator<Item = Error>> {
        match self {
            Error::LotsOfError(v) => Box::new(v.into_iter().map(|e| e.into_iter()).flatten()),
            e => Box::new(std::iter::once(e)),
        }
    }

    pub fn chain(self, other: Error) -> Error {
        Error::LotsOfError(self.into_iter().chain(other.into_iter()).collect_vec())
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
        
        match error {
            Error::Io(_) => (),
            _ => panic!("Expected Io error"),
        }
    }

    #[test]
    fn test_error_with_msg() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let error = Error::from(io_error);
        let error_with_msg = error.with_msg("Custom message");
        
        match error_with_msg {
            Error::WithMsg { msg, .. } => assert_eq!(msg, "Custom message"),
            _ => panic!("Expected WithMsg error"),
        }
    }

    #[test]
    fn test_error_with_debug_object_and_fn_name() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let error = Error::from(io_error);
        let test_obj = "test_object";
        let error_with_debug = error.with_debug_object_and_fn_name(test_obj, "test_function");
        
        match error_with_debug {
            Error::WithDebugObjAndFnName { fn_name, .. } => assert_eq!(fn_name, "test_function"),
            _ => panic!("Expected WithDebugObjAndFnName error"),
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
                match error {
                    Error::ChannelSendError(_) => (),
                    _ => panic!("Expected ChannelSendError"),
                }
            }
            Ok(_) => panic!("Expected send error"),
        }
    }

    #[test]
    fn test_error_from_vec() {
        let errors = vec![
            Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "error1")),
            Error::Io(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "error2")),
        ];
        
        let combined_error = Error::from(errors);
        match combined_error {
            Error::LotsOfError(error_vec) => assert_eq!(error_vec.len(), 2),
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
        let error = Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "test"));
        let mut iter = error.into_iter();
        
        assert!(iter.next().is_some());
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_error_into_iter_lots_of_error() {
        let errors = vec![
            Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "error1")),
            Error::Io(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "error2")),
        ];
        let combined_error = Error::from(errors);
        let iter = combined_error.into_iter();
        
        assert_eq!(iter.count(), 2);
    }

    #[test]
    fn test_error_chain() {
        let error1 = Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "error1"));
        let error2 = Error::Io(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "error2"));
        
        let chained = error1.chain(error2);
        match chained {
            Error::LotsOfError(errors) => assert_eq!(errors.len(), 2),
            _ => panic!("Expected LotsOfError"),
        }
    }

    #[test]
    fn test_error_display() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let error = Error::from(io_error);
        let error_str = error.to_string();
        assert!(error_str.contains("file not found"));
    }

    #[test]
    fn test_error_with_msg_display() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let error = Error::from(io_error);
        let error_with_msg = error.with_msg("Operation failed");
        let error_str = error_with_msg.to_string();
        
        assert!(error_str.contains("Operation failed"));
        assert!(error_str.contains("file not found"));
    }

    #[test]
    fn test_error_with_debug_display() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let error = Error::from(io_error);
        let test_obj = 42;
        let error_with_debug = error.with_debug_object_and_fn_name(test_obj, "test_function");
        let error_str = error_with_debug.to_string();
        
        assert!(error_str.contains("test_function"));
        assert!(error_str.contains("failed"));
        assert!(error_str.contains("file not found"));
    }
}