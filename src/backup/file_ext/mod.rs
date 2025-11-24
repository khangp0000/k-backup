//! File extension provider trait for backup components.
//!
//! Allows compression and encryption components to specify their file extensions
//! for building complete backup file names.

pub trait FileExtProvider {
    fn file_ext(&self) -> Option<impl AsRef<str>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestProvider {
        ext: Option<&'static str>,
    }

    impl FileExtProvider for TestProvider {
        fn file_ext(&self) -> Option<impl AsRef<str>> {
            self.ext
        }
    }

    #[test]
    fn test_file_ext_provider_some() {
        let provider = TestProvider { ext: Some("txt") };

        assert!(provider.file_ext().is_some());
        assert_eq!(provider.file_ext().unwrap().as_ref(), "txt");
    }

    #[test]
    fn test_file_ext_provider_none() {
        let provider = TestProvider { ext: None };

        assert!(provider.file_ext().is_none());
    }

    #[test]
    fn test_file_ext_provider_arc_str() {
        let ext = "json";
        let provider = TestProvider { ext: Some(ext) };

        assert!(provider.file_ext().is_some());
        assert_eq!(provider.file_ext().unwrap().as_ref(), ext);
    }

    #[test]
    fn test_file_ext_provider_multiple_calls() {
        let provider = TestProvider { ext: Some("xml") };

        // Multiple calls should return the same value
        let first_ext = provider.file_ext().unwrap();
        let second_ext = provider.file_ext().unwrap();
        assert_eq!(first_ext.as_ref(), second_ext.as_ref());
    }
}
