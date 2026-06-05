//! Glob-based file source using walkdir + globset.

use crate::config::{GlobSourceConfig, SymlinkMode};
use crate::error::ArchiveError;
use crate::pipeline::entry::{ArchiveEntry, ArchiveEntryKind};
use globset::{GlobBuilder, GlobSetBuilder};
use std::fs::{self, File};
use std::path::PathBuf;
use walkdir::WalkDir;

/// Pre-validates that the source directory exists.
pub fn validate(config: &GlobSourceConfig) -> std::result::Result<(), ArchiveError> {
    fs::metadata(&config.src_dir)
        .map(|_| ())
        .map_err(ArchiveError::from)
}

/// Returns an iterator of archive entries matching the glob patterns.
pub fn iter_entries(
    config: &GlobSourceConfig,
) -> std::result::Result<Vec<std::result::Result<ArchiveEntry, ArchiveError>>, ArchiveError> {
    // Build globset
    let mut builder = GlobSetBuilder::new();
    for pattern in &config.globset {
        let glob = GlobBuilder::new(pattern).build().map_err(|e| {
            ArchiveError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                e.to_string(),
            ))
        })?;
        builder.add(glob);
    }
    let globset = builder.build().map_err(|e| {
        ArchiveError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            e.to_string(),
        ))
    })?;

    let follow_links = config.symlink_mode == SymlinkMode::Follow;
    let mut walker = WalkDir::new(&config.src_dir).follow_links(follow_links);
    if config.max_depth > 0 {
        walker = walker.max_depth(config.max_depth);
    }

    let mut results = Vec::new();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                results.push(Err(ArchiveError::from(e)));
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
                            results.push(Err(ArchiveError::from(e)));
                            continue;
                        }
                    };
                    let dst = make_dst(config, entry.path());
                    results.push(Ok(ArchiveEntry {
                        dst,
                        kind: ArchiveEntryKind::Symlink(target),
                    }));
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
                results.push(Err(ArchiveError::from(e)));
                continue;
            }
        };

        if !globset.is_match(rel_path) {
            continue;
        }

        // Open file
        let dst = make_dst(config, entry.path());
        match File::open(entry.path()) {
            Ok(file) => results.push(Ok(ArchiveEntry {
                dst,
                kind: ArchiveEntryKind::File(file),
            })),
            Err(e) => results.push(Err(ArchiveError::from(e))),
        }
    }

    Ok(results)
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

    use tempfile::TempDir;

    fn make_config(
        dir: &std::path::Path,
        patterns: Vec<&str>,
        dst_dir: Option<&str>,
    ) -> GlobSourceConfig {
        GlobSourceConfig {
            src_dir: dir.to_path_buf(),
            dst_dir: dst_dir.map(|s| s.to_string()),
            globset: patterns.into_iter().map(|s| s.to_string()).collect(),
            symlink_mode: SymlinkMode::Follow,
            max_depth: 0,
            required: true,
        }
    }

    #[test]
    fn iter_entries_matches_correct_files() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("a.txt"), "hello").unwrap();
        fs::write(tmp.path().join("b.log"), "world").unwrap();

        let config = make_config(tmp.path(), vec!["*.txt"], None);
        let entries = iter_entries(&config).unwrap();
        let ok_entries: Vec<_> = entries.into_iter().filter_map(|r| r.ok()).collect();
        assert_eq!(ok_entries.len(), 1);
        assert_eq!(ok_entries[0].dst, PathBuf::from("a.txt"));
    }

    #[test]
    fn iter_entries_excludes_non_matching() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("a.txt"), "hello").unwrap();
        fs::write(tmp.path().join("b.log"), "world").unwrap();

        let config = make_config(tmp.path(), vec!["*.json"], None);
        let entries = iter_entries(&config).unwrap();
        let ok_entries: Vec<_> = entries.into_iter().filter_map(|r| r.ok()).collect();
        assert_eq!(ok_entries.len(), 0);
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
        let entries = iter_entries(&config).unwrap();
        let ok_entries: Vec<_> = entries.into_iter().filter_map(|r| r.ok()).collect();
        assert_eq!(ok_entries.len(), 1);
        assert_eq!(ok_entries[0].dst, PathBuf::from("backups/data.bin"));
    }
}
