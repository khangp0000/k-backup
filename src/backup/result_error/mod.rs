use std::fmt::Debug;
pub mod error;
pub mod result;

pub trait WithDebugObjectAndFnName<S: Into<String>, O: Debug + 'static> {
    fn with_debug_object_and_fn_name(self, obj: O, fn_name: S) -> Self;
}

pub trait WithMsg<S: Into<String>> {
    fn with_msg(self, msg: S) -> Self;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::result_error::error::Error;
    use crate::backup::result_error::result::Result;

    #[test]
    fn test_with_msg_trait() {
        let error = Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "test"));
        let error_with_msg = error.with_msg("Custom message");

        let error_str = error_with_msg.to_string();
        assert_eq!(error_str, "Custom message:\n  test");
    }

    #[test]
    fn test_with_debug_object_and_fn_name_trait() {
        let error = Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "test"));
        let test_obj = "test_object";
        let error_with_debug = error.with_debug_object_and_fn_name(test_obj, "test_function");

        let error_str = error_with_debug.to_string();
        assert_eq!(error_str, "\"test_object\" test_function failed:\n  test");
    }

    #[test]
    fn test_result_with_msg() {
        let result: Result<()> = Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "test",
        )));
        let result_with_msg = result.with_msg("Operation failed");

        match result_with_msg {
            Err(error) => {
                let error_str = error.to_string();
                assert_eq!(error_str, "Operation failed:\n  test");
            }
            Ok(_) => panic!("Expected error"),
        }
    }

    #[test]
    fn test_result_with_debug_object_and_fn_name() {
        let result: Result<()> = Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "test",
        )));
        let test_obj = 42;
        let result_with_debug = result.with_debug_object_and_fn_name(test_obj, "test_operation");

        match result_with_debug {
            Err(error) => {
                let error_str = error.to_string();
                assert_eq!(error_str, "42 test_operation failed:\n  test");
            }
            Ok(_) => panic!("Expected error"),
        }
    }
}
