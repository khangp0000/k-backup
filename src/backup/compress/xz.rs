use crate::backup::compress::{Compressor, CompressorBuilder};
use crate::backup::result_error::result::Result;

use bon::Builder;
use getset::Getters;
use liblzma::stream::{Check, MtStreamBuilder};
use liblzma::write::XzEncoder;
use saturating_cast::SaturatingCast;
use serde::{Deserialize, Serialize};
use validator::Validate;

use std::io::Write;
use std::num::NonZero;

/// Default compression level (balance of speed vs size)
static DEFAULT_COMPRESSION_LEVEL: u32 = 3;
/// Maximum threads to prevent resource exhaustion
static DEFAULT_MAX_PARALLELIZATION: usize = 32;

/// Configuration for XZ (LZMA) compression
///
/// XZ provides excellent compression ratios at the cost of CPU time.
/// Supports both single-threaded and multi-threaded compression modes.
/// 
/// Multi-threaded compression uses more memory but significantly improves
/// performance on multi-core systems. Thread count is automatically
/// optimized based on available CPU cores if not specified.
#[derive(Clone, Debug, Serialize, Deserialize, Validate, Builder, PartialEq, Eq, Getters)]
#[serde(deny_unknown_fields)]
#[getset(get = "pub")]
pub struct XzConfig {
    /// Compression level (0-9)
    ///
    /// - 0: Fastest, largest files
    /// - 3: Default balance (good speed/size ratio)
    /// - 9: Slowest, smallest files
    ///
    /// Higher levels use significantly more CPU time for diminishing returns.
    #[validate(range(min = 0, max = 9))]
    #[serde(default = "default_level")]
    #[builder(default = default_level())]
    level: u32,

    /// Number of compression threads
    ///
    /// - 1: Single-threaded (slower but less memory)
    /// - Auto: Half of available CPU cores, at most 32 (default)
    /// - Higher: More parallel compression (uses more memory)
    ///
    /// More threads = faster compression but higher memory usage.
    #[validate(range(min = 1))]
    #[serde(default = "default_thread")]
    #[builder(default = default_thread())]
    thread: usize,
}

fn default_level() -> u32 {
    DEFAULT_COMPRESSION_LEVEL
}

fn default_thread() -> usize {
    std::thread::available_parallelism()
        .map(NonZero::get)
        .inspect_err(|err| tracing::warn!("error getting parallelism {:?}", err))
        .map(|core| core / 2) // Use half of available cores
        .map(|t| t.max(1).min(DEFAULT_MAX_PARALLELIZATION)) // At least 1 thread
        .unwrap_or(1)
}

impl<W: Write> CompressorBuilder<W> for XzConfig {
    /// Creates an XZ compressor with the configured settings
    ///
    /// Automatically determines optimal thread count based on available CPU cores
    /// if not specified. Uses single-threaded compression for thread=1,
    /// otherwise uses multi-threaded compression for better performance.
    ///
    /// Returns configured XZ compressor
    fn build_compressor(&self, writer: W) -> Result<Compressor<W>> {
        let level = self.level;

        // Auto-detect optimal thread count if not specified
        let thread = self.thread;

        tracing::debug!(
            "Creating XZ compressor with level={}, threads={}",
            level,
            thread
        );

        if thread == 1 {
            // Single-threaded compression (less memory usage)
            Ok(Compressor::XzEncoder(XzEncoder::new(writer, level)))
        } else {
            // Multi-threaded compression (faster but more memory)
            let stream = MtStreamBuilder::new()
                .preset(level)
                .check(Check::Crc64) // Integrity checking
                .threads(thread.saturating_cast())
                .encoder()?;
            Ok(Compressor::XzEncoder(XzEncoder::new_stream(writer, stream)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_xz_config_validation() {
        // Valid configurations
        let valid_configs = vec![
            XzConfig::builder().level(0).thread(4).build(),
            XzConfig::builder().level(5).thread(4).build(),
            XzConfig::builder().level(9).thread(8).build(),
        ];

        for config in valid_configs {
            assert!(config.validate().is_ok());
        }
    }

    #[test]
    fn test_xz_config_invalid_level() {
        let config = XzConfig::builder().level(10).thread(1).build();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_xz_config_invalid_thread() {
        let config = XzConfig::builder().level(5).thread(0).build();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_build_compressor_single_thread() {
        let config = XzConfig::builder().level(6).thread(1).build();
        let writer = Cursor::new(Vec::new());
        let compressor = config.build_compressor(writer).unwrap();

        match compressor {
            Compressor::XzEncoder(_) => (),
            _ => panic!("Expected XzEncoder"),
        }
    }

    #[test]
    fn test_build_compressor_multi_thread() {
        let config = XzConfig::builder().level(6).thread(4).build();
        let writer = Cursor::new(Vec::new());
        let compressor = config.build_compressor(writer).unwrap();

        match compressor {
            Compressor::XzEncoder(_) => (),
            _ => panic!("Expected XzEncoder"),
        }
    }
}
