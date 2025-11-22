pub mod xz;

use crate::backup::file_ext::FileExtProvider;
use crate::backup::finish::Finish;
use crate::backup::result_error::result::Result;
use crate::backup::result_error::AddDebugObjectAndFnName;
use derive_more::From;
use io_enum::Write;
use liblzma::write::XzEncoder;
use serde::{Deserialize, Serialize};
use std::io;
use std::io::Write;
use std::result;

use validator::{Validate, ValidationErrors};

#[derive(Write, From)]
pub enum Compressor<W: Write> {
    None(W),
    XzEncoder(XzEncoder<W>),
}

#[derive(Clone, Default, From, Serialize, Deserialize, Debug)]
#[serde(tag = "compressor_type")]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum CompressorConfig {
    #[default]
    None,
    Xz(xz::XzConfig),
}

impl Validate for CompressorConfig {
    fn validate(&self) -> result::Result<(), ValidationErrors> {
        match self {
            CompressorConfig::None => Ok(()),
            CompressorConfig::Xz(xz) => xz.validate(),
        }
    }
}

pub trait CompressorBuilder<W: Write> {
    fn build_compressor(&self, writer: W) -> Result<Compressor<W>>;
}

impl<W: Write> Finish<W> for Compressor<W> {
    fn finish(self) -> io::Result<W> {
        match self {
            Compressor::None(w) => Ok(w),
            Compressor::XzEncoder(w) => w.finish(),
        }
    }
}

impl<W: Write> CompressorBuilder<W> for CompressorConfig {
    fn build_compressor(&self, writer: W) -> Result<Compressor<W>> {
        match self {
            CompressorConfig::None => {
                tracing::info!("Using no compression");
                Ok(Compressor::None(writer))
            }
            CompressorConfig::Xz(xz) => {
                tracing::info!("Initializing XZ compression");
                xz.build_compressor(writer)
            }
        }
        .add_debug_object_and_fn_name(self.clone(), "build_compressor")
    }
}

impl FileExtProvider for CompressorConfig {
    fn file_ext(&self) -> Option<impl AsRef<str>> {
        match self {
            CompressorConfig::None => None,
            CompressorConfig::Xz(_) => Some("xz"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::compress::xz::XzConfig;
    use std::io::Cursor;

    #[test]
    fn test_compressor_config_none() {
        let config = CompressorConfig::None;
        assert!(config.validate().is_ok());
        assert!(config.file_ext().is_none());
    }

    #[test]
    fn test_compressor_config_xz() {
        let config = CompressorConfig::Xz(XzConfig::default());
        assert!(config.validate().is_ok());
        assert!(config.file_ext().is_some());
        assert_eq!(config.file_ext().unwrap().as_ref(), "xz");
    }

    #[test]
    fn test_compressor_builder_none() {
        let config = CompressorConfig::None;
        let writer = Cursor::new(Vec::new());
        let compressor = config.build_compressor(writer).unwrap();

        match compressor {
            Compressor::None(_) => (),
            _ => panic!("Expected None compressor"),
        }
    }

    #[test]
    fn test_compressor_finish_none() {
        let writer = Cursor::new(Vec::new());
        let compressor = Compressor::None(writer);
        let result = compressor.finish();
        assert!(result.is_ok());
    }

    #[test]
    fn test_compressor_config_serialization() {
        let config = CompressorConfig::None;
        let serialized = serde_json::to_string(&config).unwrap();
        assert_eq!(serialized, "{\"compressor_type\":\"none\"}");

        let deserialized: CompressorConfig = serde_json::from_str(&serialized).unwrap();
        matches!(deserialized, CompressorConfig::None);
    }

    #[test]
    fn test_compressor_config_default() {
        let config = CompressorConfig::default();
        matches!(config, CompressorConfig::None);
    }
}
