//! Error handling and result types.
//!
//! Provides unified error types and helper traits for adding context
//! to errors throughout the backup system.

use std::borrow::Cow;

pub mod error;
pub mod result;

pub trait AddFunctionName {
    fn add_fn_name<S: Into<Cow<'static, str>>>(self, fn_name: S) -> Self;
}

pub trait AddMsg<S: Into<Cow<'static, str>>> {
    fn add_msg(self, msg: S) -> Self;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::result_error::error::Error;
    use crate::backup::result_error::result::Result;

    #[test]
    fn test_with_msg_trait() {
        let error = Error::from(std::io::Error::new(std::io::ErrorKind::NotFound, "test"));
        let error_with_msg = error.add_msg("Custom message");

        let error_str = error_with_msg.to_string();
        assert_eq!(error_str, "Custom message:\n  test");
    }

    #[test]
    fn test_result_with_msg() {
        let result: Result<()> = Err(Error::from(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "test",
        )));
        let result_with_msg = result.add_msg("Operation failed");

        match result_with_msg {
            Err(error) => {
                let error_str = error.to_string();
                assert_eq!(error_str, "Operation failed:\n  test");
            }
            Ok(_) => panic!("Expected error"),
        }
    }
}
