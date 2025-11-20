use crate::backup::compress::{Compressor, CompressorBuilder};
use crate::backup::result_error::result::Result;
use liblzma::stream::{Check, MtStreamBuilder};
use liblzma::write::XzEncoder;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::io::Write;
use std::num::NonZero;
use validator::Validate;

/// Default compression level (balance of speed vs size)
static DEFAULT_COMPRESSION_LEVEL: u32 = 3;
/// Maximum threads to prevent resource exhaustion
static DEFAULT_MAX_PARALLELIZATION: usize = 32;

/// Configuration for XZ (LZMA) compression
/// 
/// XZ provides excellent compression ratios at the cost of CPU time.
/// Supports parallel compression for better performance on multi-core systems.
#[skip_serializing_none]
#[derive(Clone, Default, Validate, Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct XzConfig {
    /// Compression level (0-9)
    /// 
    /// - 0: Fastest, largest files
    /// - 3: Default balance (good speed/size ratio)
    /// - 9: Slowest, smallest files
    /// 
    /// Higher levels use significantly more CPU time for diminishing returns.
    #[validate(range(min = 0, max = 9))]
    level: Option<u32>,
    
    /// Number of compression threads
    /// 
    /// - 1: Single-threaded (slower but less memory)
    /// - Auto: Half of available CPU cores (default)
    /// - Higher: More parallel compression (uses more memory)
    /// 
    /// More threads = faster compression but higher memory usage.
    #[validate(range(min = 1))]
    thread: Option<u32>,
}

impl<W: Write> CompressorBuilder<W> for XzConfig {
    /// Creates an XZ compressor with the configured settings
    /// 
    /// Automatically determines optimal thread count based on available CPU cores
    /// if not specified. Uses single-threaded compression for thread=1,
    /// otherwise uses multi-threaded compression for better performance.
    fn build_compressor(&self, writer: W) -> Result<Compressor<W>> {
        let level = self.level.unwrap_or(DEFAULT_COMPRESSION_LEVEL);
        
        // Auto-detect optimal thread count if not specified
        let thread = self.thread.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(NonZero::get)
                .map(|core| core / 2)  // Use half of available cores
                .map(|t| t.max(1))     // At least 1 thread
                .map(|t| t.min(DEFAULT_MAX_PARALLELIZATION) as u32)  // Cap at max
                .unwrap_or(1)
        });
        
        if thread == 1 {
            // Single-threaded compression (less memory usage)
            Ok(XzEncoder::new(writer, level).into())
        } else {
            // Multi-threaded compression (faster but more memory)
            let stream = MtStreamBuilder::new()
                .preset(level)
                .check(Check::Crc64)  // Integrity checking
                .threads(thread)
                .encoder()?;
            Ok(XzEncoder::new_stream(writer, stream).into())
        }
    }
}
