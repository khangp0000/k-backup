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
