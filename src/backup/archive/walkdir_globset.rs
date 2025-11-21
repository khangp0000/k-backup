use crate::backup::archive::{ArchiveEntry, ArchiveEntryIterable};
use crate::backup::result_error::error::Error;
use crate::backup::result_error::WithDebugObjectAndFnName;
use derive_more::{Display, From, Into};
use globset::{Glob, GlobBuilder, GlobSetBuilder};
use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize};
use serde_with::skip_serializing_none;
use std::fmt::{Debug, Formatter};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use walkdir::WalkDir;

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WalkdirAndGlobsetSource {
    src_dir: PathBuf,
    dst_dir: Option<PathBuf>,
    globset: Option<Vec<CustomDeserializedGlob>>,
}

impl WalkdirAndGlobsetSource {
    pub fn new<P: AsRef<Path>>(
        src_dir: P,
        dst_dir: Option<P>,
        globset: Option<Vec<CustomDeserializedGlob>>,
    ) -> Self {
        Self {
            src_dir: src_dir.as_ref().to_path_buf(),
            dst_dir: dst_dir.map(|p| p.as_ref().to_path_buf()),
            globset,
        }
    }
}

#[derive(Into, Clone, Serialize, From, Display)]
pub struct CustomDeserializedGlob(Glob);

impl Default for CustomDeserializedGlob {
    fn default() -> Self {
        CustomDeserializedGlob(
            GlobBuilder::new("**/*")
                .literal_separator(true)
                .build()
                .unwrap(),
        )
    }
}

impl Debug for CustomDeserializedGlob {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.to_string())
    }
}

struct CustomGlobVisitor;

impl Visitor<'_> for CustomGlobVisitor {
    type Value = CustomDeserializedGlob;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a glob pattern")
    }

    fn visit_str<E>(self, v: &str) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        GlobBuilder::new(v)
            .literal_separator(true)
            .build()
            .map(CustomDeserializedGlob::from)
            .map_err(serde::de::Error::custom)
    }
}

impl<'de> Deserialize<'de> for CustomDeserializedGlob {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_str(CustomGlobVisitor)
    }
}

impl ArchiveEntryIterable for WalkdirAndGlobsetSource {
    fn archive_entry_iterator(
        &self,
    ) -> crate::backup::result_error::result::Result<
        Box<dyn Iterator<Item = crate::backup::result_error::result::Result<ArchiveEntry>> + Send>,
    > {
        if !self.src_dir.is_dir() {
            tracing::error!(
                "Source directory does not exist or is not a directory: {:?}",
                self.src_dir
            );
            return Err(Error::Io(std::io::Error::other(
                "src_dir is not a directory",
            )));
        }

        let pattern_count = self.globset.as_ref().map(|g| g.len()).unwrap_or(1);
        tracing::info!(
            "Starting directory scan: {:?} with {} glob patterns",
            self.src_dir,
            pattern_count
        );
        tracing::debug!("Scanning directory {:?} with glob patterns", self.src_dir);

        let mut globset = GlobSetBuilder::new();

        if let Some(gs) = &self.globset {
            if gs.is_empty() {
                globset.add(CustomDeserializedGlob::default().into());
            } else {
                gs.iter().cloned().for_each(|glob| {
                    globset.add(glob.into());
                });
            }
        } else {
            globset.add(CustomDeserializedGlob::default().into());
        }

        let globset = globset.build().unwrap();
        let src_dir_clone_1 = self.src_dir.clone();
        let src_dir_clone_2 = self.src_dir.clone();
        let dst_dir = self.dst_dir.clone().unwrap_or_default();
        let self_clone = Arc::new(self.clone());

        let entries: Vec<_> = WalkDir::new(&self.src_dir)
            .follow_links(true)
            .into_iter()
            .filter(move |res| match res {
                Ok(de) => {
                    let p = de.path();
                    p.is_file()
                        && p.strip_prefix(&src_dir_clone_1)
                            .map(|p| globset.is_match(p))
                            .unwrap_or(false)
                }
                Err(_) => true,
            })
            .collect();

        tracing::info!(
            "Directory scan completed: {} files matched glob patterns",
            entries.len()
        );

        let y = entries.into_iter().map(move |res| {
            let self_clone = self_clone.clone();
            res.map(|de| {
                let entry = ArchiveEntry::new(
                    de.path().to_path_buf(),
                    dst_dir.join(de.path().strip_prefix(&src_dir_clone_2).unwrap()),
                );
                tracing::trace!("Including file: {:?} -> {:?}", entry.src, entry.dst);
                entry
            })
            .map_err(Error::from)
            .map_err(|e| e.with_debug_object_and_fn_name(self_clone, "archive_entry_iterator"))
        });

        Ok(Box::new(y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_test_files(dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir.join("subdir"))?;
        std::fs::write(dir.join("file1.txt"), "content1")?;
        std::fs::write(dir.join("file2.json"), "content2")?;
        std::fs::write(dir.join("subdir/file3.txt"), "content3")?;
        std::fs::write(dir.join("subdir/file4.log"), "content4")?;
        std::fs::write(dir.join("README.md"), "readme content")?;
        Ok(())
    }

    #[test]
    fn test_custom_deserialized_glob_default() {
        let glob = CustomDeserializedGlob::default();
        let glob_str = glob.to_string();
        assert_eq!(glob_str, "**/*");
    }

    #[test]
    fn test_custom_deserialized_glob_debug() {
        let glob = CustomDeserializedGlob::default();
        let debug_str = format!("{:?}", glob);
        assert_eq!(debug_str, "\"**/*\"");
    }

    #[test]
    fn test_custom_deserialized_glob_serialization() {
        let glob = CustomDeserializedGlob::default();
        let serialized = serde_json::to_string(&glob).unwrap();
        assert_eq!(serialized, "\"**/*\"");
    }

    #[test]
    fn test_custom_deserialized_glob_deserialization() {
        let json = "\"*.txt\"";
        let glob: CustomDeserializedGlob = serde_json::from_str(json).unwrap();
        assert_eq!(glob.to_string(), "*.txt");
    }

    #[test]
    fn test_custom_deserialized_glob_invalid_pattern() {
        let json = "\"[invalid\"";
        let result: Result<CustomDeserializedGlob, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_walkdir_globset_source_creation() {
        let src_dir = PathBuf::from("/path/to/source");
        let dst_dir = Some(PathBuf::from("backup"));
        let globset = Some(vec![CustomDeserializedGlob::default()]);

        let source = WalkdirAndGlobsetSource {
            src_dir: src_dir.clone(),
            dst_dir: dst_dir.clone(),
            globset: globset.clone(),
        };

        assert_eq!(source.src_dir, src_dir);
        assert_eq!(source.dst_dir, dst_dir);
        assert!(source.globset.is_some());
    }

    #[test]
    fn test_walkdir_globset_source_serialization() {
        let source = WalkdirAndGlobsetSource {
            src_dir: PathBuf::from("/path/to/source"),
            dst_dir: Some(PathBuf::from("backup")),
            globset: Some(vec![CustomDeserializedGlob::default()]),
        };

        let serialized = serde_json::to_string(&source).unwrap();
        let deserialized: WalkdirAndGlobsetSource = serde_json::from_str(&serialized).unwrap();

        assert_eq!(source.src_dir, deserialized.src_dir);
        assert_eq!(source.dst_dir, deserialized.dst_dir);
    }

    #[test]
    fn test_archive_entry_iterator_with_valid_directory() {
        let temp_dir = TempDir::new().unwrap();
        create_test_files(temp_dir.path()).unwrap();

        let source = WalkdirAndGlobsetSource {
            src_dir: temp_dir.path().into(),
            dst_dir: Some(PathBuf::from("backup")),
            globset: None, // Use default glob pattern
        };

        let iterator = source.archive_entry_iterator().unwrap();
        let entries: Vec<_> = iterator.collect();

        // Should find all files
        assert!(entries.len() >= 5);

        // All entries should be successful
        for entry_result in &entries {
            assert!(entry_result.is_ok());
            let entry = entry_result.as_ref().unwrap();
            // Source files are kept (not temporary)
            if let crate::backup::archive::ArchiveSource::Path(path) = &entry.src {
                assert!(path.as_ref().as_ref().is_file());
            } else {
                panic!("Expected path source");
            }
        }
    }

    #[test]
    fn test_archive_entry_iterator_with_txt_glob() {
        let temp_dir = TempDir::new().unwrap();
        create_test_files(temp_dir.path()).unwrap();

        let txt_glob = serde_json::from_str("\"**/*.txt\"").unwrap();
        let source = WalkdirAndGlobsetSource {
            src_dir: temp_dir.path().into(),
            dst_dir: Some(PathBuf::from("backup")),
            globset: Some(vec![txt_glob]),
        };

        let iterator = source.archive_entry_iterator().unwrap();
        let entries: Vec<_> = iterator.collect();

        // Should find only .txt files
        assert_eq!(entries.len(), 2); // file1.txt and subdir/file3.txt

        for entry_result in &entries {
            assert!(entry_result.is_ok());
            let entry = entry_result.as_ref().unwrap();
            if let crate::backup::archive::ArchiveSource::Path(path) = &entry.src {
                assert!(path.as_ref().as_ref().to_string_lossy().ends_with(".txt"));
            } else {
                panic!("Expected path source");
            }
        }
    }

    #[test]
    fn test_archive_entry_iterator_with_multiple_globs() {
        let temp_dir = TempDir::new().unwrap();
        create_test_files(temp_dir.path()).unwrap();

        let txt_glob: CustomDeserializedGlob = serde_json::from_str("\"**/*.txt\"").unwrap();
        let json_glob: CustomDeserializedGlob = serde_json::from_str("\"**/*.json\"").unwrap();

        let source = WalkdirAndGlobsetSource {
            src_dir: temp_dir.path().into(),
            dst_dir: Some(PathBuf::from("backup")),
            globset: Some(vec![txt_glob, json_glob]),
        };

        let iterator = source.archive_entry_iterator().unwrap();
        let entries: Vec<_> = iterator.collect();

        // Should find .txt and .json files
        assert_eq!(entries.len(), 3); // file1.txt, file2.json, subdir/file3.txt

        for entry_result in &entries {
            assert!(entry_result.is_ok());
            let entry = entry_result.as_ref().unwrap();
            if let crate::backup::archive::ArchiveSource::Path(path) = &entry.src {
                let path_str = path.as_ref().as_ref().to_string_lossy();
                assert!(path_str.ends_with(".txt") || path_str.ends_with(".json"));
            } else {
                panic!("Expected path source");
            }
        }
    }

    #[test]
    fn test_archive_entry_iterator_with_empty_globset() {
        let temp_dir = TempDir::new().unwrap();
        create_test_files(temp_dir.path()).unwrap();

        let source = WalkdirAndGlobsetSource {
            src_dir: temp_dir.path().into(),
            dst_dir: Some(PathBuf::from("backup")),
            globset: Some(vec![]), // Empty globset should use default
        };

        let iterator = source.archive_entry_iterator().unwrap();
        let entries: Vec<_> = iterator.collect();

        // Should find all files (using default glob)
        assert!(entries.len() >= 5);
    }

    #[test]
    fn test_archive_entry_iterator_with_nonexistent_directory() {
        let source = WalkdirAndGlobsetSource {
            src_dir: PathBuf::from("/nonexistent/directory"),
            dst_dir: Some(PathBuf::from("backup")),
            globset: None,
        };

        let result = source.archive_entry_iterator();
        assert!(result.is_err());
    }

    #[test]
    fn test_archive_entry_iterator_with_file_as_src_dir() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("not_a_directory.txt");
        std::fs::write(&file_path, "content").unwrap();

        let source = WalkdirAndGlobsetSource {
            src_dir: file_path,
            dst_dir: Some(PathBuf::from("backup")),
            globset: None,
        };

        let result = source.archive_entry_iterator();
        assert!(result.is_err());
    }

    #[test]
    fn test_archive_entry_iterator_dst_dir_none() {
        let temp_dir = TempDir::new().unwrap();
        create_test_files(temp_dir.path()).unwrap();

        let source = WalkdirAndGlobsetSource {
            src_dir: temp_dir.path().into(),
            dst_dir: None, // Should use empty path as default
            globset: None,
        };

        let iterator = source.archive_entry_iterator().unwrap();
        let entries: Vec<_> = iterator.collect();

        assert!(!entries.is_empty());

        // Check that dst paths don't have a prefix
        for entry_result in &entries {
            let entry = entry_result.as_ref().unwrap();
            let dst_str = entry.dst.as_ref().as_ref().to_string_lossy();
            // Should be relative paths without "backup/" prefix
            assert!(!dst_str.starts_with("backup/"));
        }
    }

    #[test]
    fn test_walkdir_globset_source_debug() {
        let source = WalkdirAndGlobsetSource {
            src_dir: PathBuf::from("/path/to/source"),
            dst_dir: Some(PathBuf::from("backup")),
            globset: Some(vec![CustomDeserializedGlob::default()]),
        };

        let debug_str = format!("{:?}", source);
        assert_eq!(debug_str, "WalkdirAndGlobsetSource { src_dir: \"/path/to/source\", dst_dir: Some(\"backup\"), globset: Some([\"**/*\"]) }");
    }
}
