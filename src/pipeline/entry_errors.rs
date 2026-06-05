//! Collects non-fatal errors grouped by source config during archive creation.

use crate::config::ArchiveEntryConfig;
use crate::error::Error;
use std::fmt;
use std::sync::Arc;

const MAX_ERRORS: usize = 10;

/// A single entry error tied to the source config that produced it.
pub struct EntryError {
    pub source: Arc<ArchiveEntryConfig>,
    pub error: Error,
}

/// Collection of non-fatal entry errors, capped at MAX_ERRORS.
/// Required errors are never dropped regardless of cap.
pub struct EntryErrors {
    pub errors: Vec<EntryError>,
    pub truncated: bool,
}

impl EntryErrors {
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            truncated: false,
        }
    }

    pub fn push(&mut self, entry: EntryError) {
        if self.errors.len() < MAX_ERRORS || entry.source.is_required() {
            self.errors.push(entry);
        } else {
            self.truncated = true;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn has_required_failure(&self) -> bool {
        self.errors.iter().any(|e| e.source.is_required())
    }

    pub fn merge(&mut self, other: EntryErrors) {
        for entry in other.errors {
            self.push(entry);
        }
        self.truncated |= other.truncated;
    }
}

impl Default for EntryErrors {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for EntryErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, entry) in self.errors.iter().enumerate() {
            if i > 0 {
                write!(f, "\n\n")?;
            }
            write!(f, "[{:?}]\n  {}", entry.source, entry.error)?;
        }
        if self.truncated {
            write!(f, "\n\n... (additional errors truncated)")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Base64SourceConfig, GlobSourceConfig, SymlinkMode};
    use std::io;

    fn required_source() -> Arc<ArchiveEntryConfig> {
        Arc::new(ArchiveEntryConfig::Base64(Base64SourceConfig {
            content: crate::config::Base64Bytes::new(b"a".to_vec()),
            dst: "a.txt".into(),
        }))
    }

    fn optional_source() -> Arc<ArchiveEntryConfig> {
        Arc::new(ArchiveEntryConfig::Glob(GlobSourceConfig {
            src_dir: "/tmp".into(),
            dst_dir: None,
            globset: crate::config::CompiledGlobSet::new(vec!["*".into()]).unwrap(),
            symlink_mode: SymlinkMode::Follow,
            max_depth: 0,
            required: false,
        }))
    }

    fn make_entry_error(source: Arc<ArchiveEntryConfig>) -> EntryError {
        EntryError {
            source,
            error: Error::from(io::Error::other("test error")),
        }
    }

    #[test]
    fn push_respects_max_errors_cap() {
        let mut ee = EntryErrors::new();
        for _ in 0..15 {
            ee.push(make_entry_error(optional_source()));
        }
        assert_eq!(ee.errors.len(), MAX_ERRORS);
        assert!(ee.truncated);
    }

    #[test]
    fn push_never_drops_required_errors() {
        let mut ee = EntryErrors::new();
        // Fill to cap with optional
        for _ in 0..MAX_ERRORS {
            ee.push(make_entry_error(optional_source()));
        }
        // Required should still be accepted
        ee.push(make_entry_error(required_source()));
        assert_eq!(ee.errors.len(), MAX_ERRORS + 1);
    }

    #[test]
    fn has_required_failure_true() {
        let mut ee = EntryErrors::new();
        ee.push(make_entry_error(required_source()));
        assert!(ee.has_required_failure());
    }

    #[test]
    fn has_required_failure_false() {
        let mut ee = EntryErrors::new();
        ee.push(make_entry_error(optional_source()));
        assert!(!ee.has_required_failure());
    }

    #[test]
    fn merge_combines_two() {
        let mut a = EntryErrors::new();
        a.push(make_entry_error(optional_source()));
        let mut b = EntryErrors::new();
        b.push(make_entry_error(required_source()));
        a.merge(b);
        assert_eq!(a.errors.len(), 2);
    }

    #[test]
    fn truncated_flag_set_when_cap_exceeded() {
        let mut ee = EntryErrors::new();
        for _ in 0..MAX_ERRORS + 1 {
            ee.push(make_entry_error(optional_source()));
        }
        assert!(ee.truncated);
    }

    #[test]
    fn display_format() {
        let mut ee = EntryErrors::new();
        ee.push(make_entry_error(optional_source()));
        ee.truncated = true;
        let display = format!("{}", ee);
        assert!(display.contains("test error"));
        assert!(display.contains("additional errors truncated"));
    }
}
