use crate::backup::result_error::error::Error;
use crate::backup::result_error::{WithDebugObjectAndFnName, WithMsg};
use std::fmt::Debug;

pub type Result<T> = std::result::Result<T, Error>;

impl<S: Into<String>, O: Debug + Send + 'static, R> WithDebugObjectAndFnName<S, O> for Result<R> {
    fn with_debug_object_and_fn_name(self, obj: O, fn_name: S) -> Self {
        self.map_err(|e| e.with_debug_object_and_fn_name(obj, fn_name))
    }
}

impl<R, S: Into<String>> WithMsg<S> for Result<R> {
    fn with_msg(self, msg: S) -> Self {
        self.map_err(|e| e.with_msg(msg))
    }
}

pub fn convert_error_vec(errors: Vec<Error>) -> Result<()> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.into())
    }
}
