use std::sync::Arc;

pub trait FileExtProvider {
    fn file_ext(&self) -> Option<Arc<str>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestProvider {
        ext: Option<Arc<str>>,
    }

    impl FileExtProvider for TestProvider {
        fn file_ext(&self) -> Option<Arc<str>> {
            self.ext.clone()
        }
    }

    #[test]
    fn test_file_ext_provider_some() {
        let provider = TestProvider {
            ext: Some("txt".into()),
        };
        
        assert_eq!(provider.file_ext(), Some("txt".into()));
    }

    #[test]
    fn test_file_ext_provider_none() {
        let provider = TestProvider { ext: None };
        
        assert_eq!(provider.file_ext(), None);
    }

    #[test]
    fn test_file_ext_provider_arc_str() {
        let ext: Arc<str> = "json".into();
        let provider = TestProvider {
            ext: Some(ext.clone()),
        };
        
        assert_eq!(provider.file_ext(), Some(ext));
    }

    #[test]
    fn test_file_ext_provider_multiple_calls() {
        let provider = TestProvider {
            ext: Some("xml".into()),
        };
        
        // Multiple calls should return the same value
        assert_eq!(provider.file_ext(), provider.file_ext());
    }
}