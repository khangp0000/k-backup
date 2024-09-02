use crate::backup::compress::{Compressor, CompressorBuilder};
use crate::backup::result_error::result::Result;
use liblzma::stream::{Check, MtStreamBuilder};
use liblzma::write::XzEncoder;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::io::Write;
use std::num::NonZero;
use validator::Validate;

static DEFAULT_COMPRESSION_LEVEL: u32 = 3;
static DEFAULT_MAX_PARALLELIZATION: usize = 32;

#[skip_serializing_none]
#[derive(Clone, Default, Validate, Serialize, Deserialize, Debug)]
pub struct XzConfig {
    #[validate(range(min = 0, max = 9))]
    level: Option<u32>,
    #[validate(range(min = 1))]
    thread: Option<u32>,
}

impl<W: Write> CompressorBuilder<W> for XzConfig {
    fn build_compressor(&self, writer: W) -> Result<Compressor<W>> {
        let level = self.level.unwrap_or(DEFAULT_COMPRESSION_LEVEL);
        let thread = self.thread.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(NonZero::get)
                .map(|core| core / 2)
                .map(|t| t.max(1))
                .map(|t| t.min(DEFAULT_MAX_PARALLELIZATION) as u32)
                .unwrap_or(1)
        });
        if thread == 1 {
            Ok(XzEncoder::new(writer, level).into())
        } else {
            let stream = MtStreamBuilder::new()
                .preset(level)
                .check(Check::Crc64)
                .threads(thread)
                .encoder()?;
            Ok(XzEncoder::new_stream(writer, stream).into())
        }
    }
}
