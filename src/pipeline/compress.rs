//! XZ compression Write wrapper.

use crate::error::CompressError;
use crate::pipeline::FinishableWrite;
use liblzma::stream::{MtStreamBuilder, Stream};
use liblzma::write::XzEncoder;
use std::io::Write;

/// XZ encoder that implements FinishableWrite.
struct FinishableXzEncoder<W: Write + Send>(Option<XzEncoder<W>>);

impl<W: Write + Send> Write for FinishableXzEncoder<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.as_mut().unwrap().write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.as_mut().unwrap().flush()
    }
}

impl FinishableWrite for FinishableXzEncoder<Box<dyn FinishableWrite>> {
    fn finish(mut self: Box<Self>) -> std::io::Result<()> {
        if let Some(encoder) = self.0.take() {
            let inner = encoder.finish()?;
            inner.finish()?;
        }
        Ok(())
    }
}

/// Wraps a writer with compression. Returns Box<dyn FinishableWrite>.
pub fn wrap_writer(
    config: &crate::config::CompressorConfig,
    writer: Box<dyn FinishableWrite>,
) -> std::result::Result<Box<dyn FinishableWrite>, CompressError> {
    match config {
        crate::config::CompressorConfig::None => Ok(writer),
        crate::config::CompressorConfig::Xz { level, thread } => {
            let threads = thread.unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1)
            });

            let stream = if threads > 1 {
                MtStreamBuilder::new()
                    .preset(*level)
                    .threads(threads as u32)
                    .check(liblzma::stream::Check::Crc64)
                    .encoder()?
            } else {
                Stream::new_easy_encoder(*level, liblzma::stream::Check::Crc64)?
            };

            Ok(Box::new(FinishableXzEncoder(Some(XzEncoder::new_stream(
                writer, stream,
            )))))
        }
    }
}
