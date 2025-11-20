use age::stream::StreamWriter;
use liblzma::write::XzEncoder;
use std::io::{Error, Write};

impl<W: Write> Finish<W> for StreamWriter<W> {
    fn finish(self) -> Result<W, Error> {
        self.finish()
    }
}

pub trait Finish<O> {
    fn finish(self) -> Result<O, Error>;
}

impl<W: Write> Finish<W> for XzEncoder<W> {
    fn finish(self) -> Result<W, Error> {
        self.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct TestFinisher {
        inner: Cursor<Vec<u8>>,
        should_fail: bool,
    }

    impl Finish<Cursor<Vec<u8>>> for TestFinisher {
        fn finish(self) -> Result<Cursor<Vec<u8>>, Error> {
            if self.should_fail {
                Err(Error::new(std::io::ErrorKind::Other, "Test failure"))
            } else {
                Ok(self.inner)
            }
        }
    }

    #[test]
    fn test_finish_trait_success() {
        let cursor = Cursor::new(vec![1, 2, 3]);
        let finisher = TestFinisher {
            inner: cursor,
            should_fail: false,
        };
        
        let result = finisher.finish();
        assert!(result.is_ok());
        
        let returned_cursor = result.unwrap();
        assert_eq!(returned_cursor.get_ref(), &vec![1, 2, 3]);
    }

    #[test]
    fn test_finish_trait_failure() {
        let cursor = Cursor::new(vec![1, 2, 3]);
        let finisher = TestFinisher {
            inner: cursor,
            should_fail: true,
        };
        
        let result = finisher.finish();
        assert!(result.is_err());
        
        let error = result.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_eq!(error.to_string(), "Test failure");
    }

    #[test]
    fn test_xz_encoder_finish_impl() {
        let cursor = Cursor::new(Vec::new());
        let encoder = XzEncoder::new(cursor, 1);
        
        // This should compile and work with the Finish trait
        let result = encoder.finish();
        assert!(result.is_ok());
    }

    // Note: StreamWriter test would require more complex setup with Age encryption
    // and is better tested through integration tests
}