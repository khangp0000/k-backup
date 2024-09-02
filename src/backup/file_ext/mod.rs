use std::sync::Arc;

pub trait FileExtProvider {
    fn file_ext(&self) -> Option<Arc<str>>;
}
