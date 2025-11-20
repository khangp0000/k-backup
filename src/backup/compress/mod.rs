pub mod xz;

use crate::backup::file_ext::FileExtProvider;
use crate::backup::finish::Finish;
use crate::backup::result_error::result::Result;
use crate::backup::result_error::WithDebugObjectAndFnName;
use derive_more::From;
use io_enum::Write;
use liblzma::write::XzEncoder;
use serde::{Deserialize, Serialize};
use std::io;
use std::io::Write;
use std::result;
use std::sync::{Arc, OnceLock};
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
            CompressorConfig::None => Ok(Compressor::None(writer)),
            CompressorConfig::Xz(xz) => xz.build_compressor(writer),
        }
        .with_debug_object_and_fn_name(self.clone(), "build_compressor")
    }
}

static XZ_FILE_EXT: OnceLock<Arc<str>> = OnceLock::new();
impl FileExtProvider for CompressorConfig {
    fn file_ext(&self) -> Option<Arc<str>> {
        match self {
            CompressorConfig::None => None,
            CompressorConfig::Xz(_) => Some(XZ_FILE_EXT.get_or_init(|| "xz".into()).clone()),
        }
    }
}
