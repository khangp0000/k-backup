use crate::backup::archive::{ArchiveEntry, ArchiveEntryIterable};
use crate::backup::result_error::error::Error;
use crate::backup::result_error::WithDebugObjectAndFnName;
use derive_more::{Display, From, Into};
use globset::{Glob, GlobBuilder, GlobSetBuilder};
use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize};
use serde_with::skip_serializing_none;
use std::fmt::{Debug, Formatter};
use std::path::Path;
use std::sync::Arc;
use walkdir::WalkDir;

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WalkdirAndGlobsetSource {
    src_dir: Arc<Path>,
    dst_dir: Option<Arc<Path>>,
    globset: Option<Vec<CustomDeserializedGlob>>,
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

impl<'de> Visitor<'de> for CustomGlobVisitor {
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
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "src_dir is not a directory",
            )));
        }

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
        let dst_dir = self.dst_dir.clone().unwrap_or(Path::new("").into());
        let self_clone = Arc::new(self.clone());

        let y = WalkDir::new(self.src_dir.as_ref())
            .follow_links(true)
            .into_iter()
            .filter(move |res| match res {
                Ok(de) => {
                    let p = de.path();
                    p.is_file()
                        && p.strip_prefix(src_dir_clone_1.as_ref())
                            .map(|p| globset.is_match(p))
                            .unwrap_or(false)
                }
                Err(_) => true,
            })
            .map(move |res| {
                let self_clone = self_clone.clone();
                res.map(|de| {
                    ArchiveEntry::keep_src(
                        de.path().to_path_buf(),
                        dst_dir.join(de.path().strip_prefix(src_dir_clone_2.as_ref()).unwrap()),
                    )
                })
                .map_err(Error::from)
                .map_err(|e| e.with_debug_object_and_fn_name(self_clone, "archive_entry_iterator"))
            });

        Ok(Box::new(y))
    }
}
