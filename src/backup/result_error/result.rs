use crate::backup::result_error::error::Error;
use crate::backup::result_error::{AddFunctionName, AddMsg};
use std::borrow::Cow;

pub type Result<T> = std::result::Result<T, Error>;

impl<R> AddFunctionName for Result<R> {
    fn add_fn_name<S: Into<Cow<'static, str>>>(self, fn_name: S) -> Self {
        self.map_err(|e| e.add_fn_name(fn_name))
    }
}

impl<R, S: Into<Cow<'static, str>>> AddMsg<S> for Result<R> {
    fn add_msg(self, msg: S) -> Self {
        self.map_err(|e| e.add_msg(msg))
    }
}

pub fn convert_error_vec(errors: Vec<Error>) -> Result<()> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::result_error::error::ErrorInternal;

    #[test]
    fn test_result_with_msg_ok() {
        let result: Result<i32> = Ok(42);
        let result_with_msg = result.add_msg("This should not affect Ok");

        assert_eq!(result_with_msg.unwrap(), 42);
    }

    #[test]
    fn test_result_with_msg_err() {
        let result: Result<i32> = Err(Error::from(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "test",
        )));
        let result_with_msg = result.add_msg("Custom message");

        if let Err(err_internal) = &result_with_msg {
            match err_internal.inner() {
                ErrorInternal::WithMsg { msg, .. } => assert_eq!(msg, "Custom message"),
                _ => panic!("Expected WithMsg error"),
            }
        }
    }

    #[test]
    fn test_convert_error_vec_empty() {
        let errors: Vec<Error> = vec![];
        let result = convert_error_vec(errors);
        assert!(result.is_ok());
    }

    #[test]
    fn test_convert_error_vec_with_errors() {
        let errors = vec![
            Error::from(std::io::Error::new(std::io::ErrorKind::NotFound, "error1")),
            Error::from(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "error2",
            )),
        ];

        let result = convert_error_vec(errors);
        assert!(result.is_err());

        if let Err(err_internal) = &result {
            match err_internal.inner() {
                ErrorInternal::LotsOfError(error_vec) => assert_eq!(error_vec.len(), 2),
                _ => panic!("Expected LotsOfError"),
            }
        }
    }

    #[test]
    fn test_convert_error_vec_single_error() {
        let errors = vec![Error::from(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "single error",
        ))];

        let result = convert_error_vec(errors);
        assert!(result.is_err());

        if let Err(err_internal) = &result {
            match err_internal.inner() {
                ErrorInternal::LotsOfError(error_vec) => assert_eq!(error_vec.len(), 1),
                _ => panic!("Expected LotsOfError"),
            }
        }
    }

    #[test]
    fn test_result_type_alias() {
        // Test that our Result type alias works correctly
        let ok_result: Result<String> = Ok("success".to_string());
        let err_result: Result<String> = Err(Error::from(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "test",
        )));

        assert!(ok_result.is_ok());
        assert!(err_result.is_err());
    }

    #[test]
    fn test_chained_error_handling() {
        let result: Result<i32> = Err(Error::from(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "original",
        )));
        let result = result.add_msg("First message").add_fn_name("test_function");

        if let Err(err_internal) = &result {
            match err_internal.inner() {
                ErrorInternal::WithFnName { error, fn_name, .. } => {
                    assert_eq!(fn_name, "test_function");
                    match error.inner() {
                        ErrorInternal::WithMsg { msg, .. } => assert_eq!(msg, "First message"),
                        _ => panic!("Expected WithMsg error inside"),
                    }
                }
                _ => panic!("Expected LotsOfError"),
            }
        }
    }
}
