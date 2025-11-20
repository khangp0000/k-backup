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
        
        tracing::debug!("Creating XZ compressor with level={}, threads={}", level, thread);
        
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_xz_config_default() {
        let config = XzConfig::default();
        assert!(config.level.is_none());
        assert!(config.thread.is_none());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_xz_config_validation() {
        // Valid configurations
        let valid_configs = vec![
            XzConfig { level: Some(0), thread: Some(1) },
            XzConfig { level: Some(5), thread: Some(4) },
            XzConfig { level: Some(9), thread: Some(8) },
        ];
        
        for config in valid_configs {
            assert!(config.validate().is_ok());
        }
    }

    #[test]
    fn test_xz_config_invalid_level() {
        let config = XzConfig { level: Some(10), thread: Some(1) };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_xz_config_invalid_thread() {
        let config = XzConfig { level: Some(5), thread: Some(0) };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_build_compressor_single_thread() {
        let config = XzConfig { level: Some(6), thread: Some(1) };
        let writer = Cursor::new(Vec::new());
        let compressor = config.build_compressor(writer).unwrap();
        
        match compressor {
            Compressor::XzEncoder(_) => (),
            _ => panic!("Expected XzEncoder"),
        }
    }

    #[test]
    fn test_build_compressor_multi_thread() {
        let config = XzConfig { level: Some(6), thread: Some(4) };
        let writer = Cursor::new(Vec::new());
        let compressor = config.build_compressor(writer).unwrap();
        
        match compressor {
            Compressor::XzEncoder(_) => (),
            _ => panic!("Expected XzEncoder"),
        }
    }

    #[test]
    fn test_build_compressor_auto_thread() {
        let config = XzConfig { level: Some(6), thread: None };
        let writer = Cursor::new(Vec::new());
        let compressor = config.build_compressor(writer).unwrap();
        
        match compressor {
            Compressor::XzEncoder(_) => (),
            _ => panic!("Expected XzEncoder"),
        }
    }

    #[test]
    fn test_xz_config_serialization() {
        let config = XzConfig { level: Some(6), thread: Some(4) };
        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: XzConfig = serde_json::from_str(&serialized).unwrap();
        
        assert_eq!(config.level, deserialized.level);
        assert_eq!(config.thread, deserialized.thread);
    }

    #[test]
    fn test_compression_level_defaults() {
        let config = XzConfig::default();
        let writer = Cursor::new(Vec::new());
        let _compressor = config.build_compressor(writer).unwrap();
        // Should use DEFAULT_COMPRESSION_LEVEL internally
    }

    #[test]
    fn test_thread_count_calculation() {
        // Test that thread count calculation doesn't panic
        let config = XzConfig { level: None, thread: None };
        let writer = Cursor::new(Vec::new());
        let _compressor = config.build_compressor(writer).unwrap();
    }
}