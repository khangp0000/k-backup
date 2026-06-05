//! Glob-based file source using walkdir + globset.

use crate::config::{GlobSourceConfig, SymlinkMode};
use crate::error::ArchiveError;
use crate::pipeline::entry::{ArchiveEntry, ArchiveEntryKind};
use std::fs::{self, File};
use std::path::PathBuf;
use walkdir::WalkDir;

/// Pre-validates that the source directory exists.
pub fn validate(config: &GlobSourceConfig) -> std::result::Result<(), ArchiveError> {
    fs::metadata(&config.src_dir)
        .map(|_| ())
        .map_err(ArchiveError::from)
}

/// Walks the directory and sends matching entries directly through the channel.
/// Returns errors encountered during traversal.
/// Breaks early if the channel closes (tar writer stopped).
pub fn send_entries(
    config: &GlobSourceConfig,
    tx: &std::sync::mpsc::SyncSender<ArchiveEntry>,
) -> std::ops::ControlFlow<Vec<ArchiveError>, Vec<ArchiveError>> {
    let follow_links = config.symlink_mode == SymlinkMode::Follow;
    let mut walker = WalkDir::new(&config.src_dir).follow_links(follow_links);
    if config.max_depth > 0 {
        walker = walker.max_depth(config.max_depth);
    }

    let mut errors = Vec::new();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                errors.push(ArchiveError::from(e));
                continue;
            }
        };

        let file_type = entry.file_type();

        // Handle symlinks based on mode
        if file_type.is_symlink() {
            match config.symlink_mode {
                SymlinkMode::Follow => unreachable!(), // follow_links=true resolves these
                SymlinkMode::Preserve => {
                    let target = match fs::read_link(entry.path()) {
                        Ok(t) => t,
                        Err(e) => {
                            errors.push(ArchiveError::from(e));
                            continue;
                        }
                    };
                    let dst = make_dst(config, entry.path());
                    if tx
                        .send(ArchiveEntry {
                            dst,
                            kind: ArchiveEntryKind::Symlink(target),
                        })
                        .is_err()
                    {
                        return std::ops::ControlFlow::Break(errors);
                    }
                    continue;
                }
                SymlinkMode::Skip => continue,
            }
        }

        if !file_type.is_file() {
            continue;
        }

        // Check glob match
        let rel_path = match entry.path().strip_prefix(&config.src_dir) {
            Ok(p) => p,
            Err(e) => {
                errors.push(ArchiveError::from(e));
                continue;
            }
        };

        if !config.globset.is_match(rel_path) {
            continue;
        }

        // Open file and send immediately
        let dst = make_dst(config, entry.path());
        match File::open(entry.path()) {
            Ok(file) => {
                if tx
                    .send(ArchiveEntry {
                        dst,
                        kind: ArchiveEntryKind::File(file),
                    })
                    .is_err()
                {
                    return std::ops::ControlFlow::Break(errors);
                }
            }
            Err(e) => errors.push(ArchiveError::from(e)),
        }
    }

    std::ops::ControlFlow::Continue(errors)
}

fn make_dst(config: &GlobSourceConfig, path: &std::path::Path) -> PathBuf {
    let rel = path.strip_prefix(&config.src_dir).unwrap_or(path);
    match &config.dst_dir {
        Some(prefix) => PathBuf::from(prefix).join(rel),
        None => rel.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::sync_channel;

    use tempfile::TempDir;

    fn make_config(
        dir: &std::path::Path,
        patterns: Vec<&str>,
        dst_dir: Option<&str>,
    ) -> GlobSourceConfig {
        use crate::config::CompiledGlobSet;
        GlobSourceConfig {
            src_dir: dir.to_path_buf(),
            dst_dir: dst_dir.map(|s| s.to_string()),
            globset: CompiledGlobSet::new(patterns.into_iter().map(|s| s.to_string()).collect())
                .unwrap(),
            symlink_mode: SymlinkMode::Follow,
            max_depth: 0,
            required: true,
        }
    }

    fn collect_entries(config: &GlobSourceConfig) -> (Vec<ArchiveEntry>, Vec<ArchiveError>) {
        use std::ops::ControlFlow;
        let (tx, rx) = sync_channel(64);
        let errors = match send_entries(config, &tx) {
            ControlFlow::Continue(e) | ControlFlow::Break(e) => e,
        };
        drop(tx);
        (rx.into_iter().collect(), errors)
    }

    #[test]
    fn send_entries_matches_correct_files() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("a.txt"), "hello").unwrap();
        fs::write(tmp.path().join("b.log"), "world").unwrap();

        let config = make_config(tmp.path(), vec!["*.txt"], None);
        let (entries, _) = collect_entries(&config);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].dst, PathBuf::from("a.txt"));
    }

    #[test]
    fn send_entries_excludes_non_matching() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("a.txt"), "hello").unwrap();
        fs::write(tmp.path().join("b.log"), "world").unwrap();

        let config = make_config(tmp.path(), vec!["*.json"], None);
        let (entries, _) = collect_entries(&config);
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn validate_fails_on_nonexistent_dir() {
        let config = make_config(
            std::path::Path::new("/nonexistent/path/xyz"),
            vec!["*"],
            None,
        );
        assert!(validate(&config).is_err());
    }

    #[test]
    fn dst_dir_prefix_is_applied() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("data.bin"), "bytes").unwrap();

        let config = make_config(tmp.path(), vec!["*.bin"], Some("backups/"));
        let (entries, _) = collect_entries(&config);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].dst, PathBuf::from("backups/data.bin"));
    }
}
