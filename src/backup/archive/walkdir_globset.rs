use crate::backup::archive::{ArchiveEntry, ArchiveEntryIterable};
use crate::backup::function_path;
use crate::backup::result_error::error::Error;
use crate::backup::result_error::result::Result;
use crate::backup::result_error::AddFunctionName;
use crate::backup::result_error::AddMsg;
use crate::backup::validate::validate_dir_exist;
use derive_ctor::ctor;
use derive_more::{Display, From};
use dyn_iter::{DynIter, IntoDynIterator};
use function_name::named;
use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};
use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt::{Debug, Formatter};
use std::path::{Path, PathBuf};
use validator::Validate;
use walkdir::{DirEntry, WalkDir};

#[derive(Clone, Debug, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
#[derive(ctor)]
#[ctor(pub new)]
pub struct WalkdirAndGlobsetSource {
    #[ctor(into)]
    #[validate(custom(function = validate_dir_exist))]
    src_dir: PathBuf,
    #[ctor(into)]
    #[serde(default)]
    dst_dir: PathBuf,
    #[ctor(into)]
    #[serde(default = "default_globset")]
    globset: Vec<CustomDeserializedGlob>,
}

fn default_globset() -> Vec<CustomDeserializedGlob> {
    vec![CustomDeserializedGlob::default()]
}

#[derive(Clone, Debug, Serialize, From, Display, ctor)]
#[ctor(pub new)]
#[serde(transparent)]
pub struct CustomDeserializedGlob {
    #[ctor(into)]
    glob: Glob,
}

impl Default for CustomDeserializedGlob {
    fn default() -> Self {
        GlobBuilder::new("**/*")
            .literal_separator(true)
            .build()
            .unwrap()
            .into()
    }
}

struct CustomGlobVisitor;

impl Visitor<'_> for CustomGlobVisitor {
    type Value = CustomDeserializedGlob;

    fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
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
    #[named]
    fn archive_entry_iterator<'a>(&self) -> Result<DynIter<'a, Result<ArchiveEntry>>> {
        if !self.src_dir.is_dir() {
            tracing::error!(
                "Source directory does not exist or is not a directory: {:?}",
                self.src_dir
            );
            return Err(Error::from(std::io::Error::other(
                "src_dir is not a directory",
            )));
        }

        let pattern_count = self.globset.len();
        tracing::info!(
            "Starting directory scan: {:?} with {} glob patterns",
            self.src_dir,
            pattern_count
        );
        tracing::debug!("Scanning directory {:?} with glob patterns", self.src_dir);

        let mut globset = GlobSetBuilder::new();

        let gs = &self.globset;
        if gs.is_empty() {
            globset.add(CustomDeserializedGlob::default().glob);
        } else {
            gs.iter().map(|g| g.glob.clone()).for_each(|g| {
                globset.add(g);
            });
        }

        let globset = globset.build().unwrap();
        let src_dir = self.src_dir.to_path_buf();
        let dst_dir = self.dst_dir.to_path_buf();

        let entries = WalkDir::new(&self.src_dir)
            .follow_links(true)
            .into_iter()
            .filter_map(move |res| match res {
                Ok(de) => process_dir_entry(de, &src_dir, &dst_dir, &globset),
                Err(e) => Some(Err(e.into())),
            })
            .map(move |res| res.add_fn_name(function_path!()));

        Ok(entries.into_dyn_iter())
    }
}

fn process_dir_entry<P1: AsRef<Path>, P2: AsRef<Path>>(
    de: DirEntry,
    base_src_dir: P1,
    base_dst_dir: P2,
    globset: &GlobSet,
) -> Option<Result<ArchiveEntry>> {
    let p = de.into_path();
    let res = if p.is_file() {
        tracing::debug!("Checking glob path {:?}", p);
        match p.strip_prefix(base_src_dir.as_ref()) {
            Ok(stripped_path) => {
                if globset.is_match(stripped_path) {
                    Ok(base_dst_dir.as_ref().join(stripped_path))
                } else {
                    tracing::trace!("Skipping {:?}, glob not match", p);
                    return None;
                }
            }
            Err(e) => Err(Error::from(e).add_msg(format!(
                "Stripping {:?} from {:?} failed",
                base_src_dir.as_ref(),
                p
            ))),
        }
    } else {
        tracing::trace!("Skipping {:?} not a file", p);
        return None;
    };

    Some(res.map(|dst| {
        let entry = ArchiveEntry::new_path(p, dst);
        tracing::trace!("Including file: {:?} -> {:?}", entry.src, entry.dst);
        entry
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
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
        assert_eq!(debug_str, "CustomDeserializedGlob { glob: Glob(\"**/*\") }");
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
        let result = serde_json::from_str::<CustomDeserializedGlob>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_walkdir_globset_source_serialization() {
        let source = WalkdirAndGlobsetSource::new(
            "/path/to/source",
            "backup",
            vec![CustomDeserializedGlob::default()],
        );

        let serialized = serde_json::to_string(&source).unwrap();
        let deserialized: WalkdirAndGlobsetSource = serde_json::from_str(&serialized).unwrap();

        assert_eq!(source.src_dir, deserialized.src_dir);
        assert_eq!(source.dst_dir, deserialized.dst_dir);
    }

    #[test]
    fn test_archive_entry_iterator_with_valid_directory() {
        let temp_dir = TempDir::new().unwrap();
        create_test_files(temp_dir.path()).unwrap();

        let source = WalkdirAndGlobsetSource::new(temp_dir.path(), "backup", vec![]);

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
        let source = WalkdirAndGlobsetSource::new(temp_dir.path(), "backup", vec![txt_glob]);

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

        let source =
            WalkdirAndGlobsetSource::new(temp_dir.path(), "backup", vec![txt_glob, json_glob]);

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

        let source = WalkdirAndGlobsetSource::new(temp_dir.path(), "backup", vec![]);

        let iterator = source.archive_entry_iterator().unwrap();
        let entries: Vec<_> = iterator.collect();

        // Should find all files (using default glob)
        assert!(entries.len() >= 5);
    }

    #[test]
    fn test_archive_entry_iterator_with_nonexistent_directory() {
        let source = WalkdirAndGlobsetSource::new("/nonexistent/directory", "backup", vec![]);

        let result = source.archive_entry_iterator();
        assert!(result.is_err());
    }

    #[test]
    fn test_archive_entry_iterator_with_file_as_src_dir() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("not_a_directory.txt");
        std::fs::write(&file_path, "content").unwrap();

        let source = WalkdirAndGlobsetSource::new(file_path, "backup", vec![]);

        let result = source.archive_entry_iterator();
        assert!(result.is_err());
    }

    #[test]
    fn test_walkdir_globset_source_debug() {
        let source = WalkdirAndGlobsetSource::new(
            "/path/to/source",
            "backup",
            vec![CustomDeserializedGlob::default()],
        );

        let debug_str = format!("{:?}", source);
        assert_eq!(debug_str, "WalkdirAndGlobsetSource { src_dir: \"/path/to/source\", dst_dir: \"backup\", globset: [CustomDeserializedGlob { glob: Glob(\"**/*\") }] }");
    }
}
