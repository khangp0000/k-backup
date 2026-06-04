//! Archive creation pipeline: sources → tar → compress → encrypt.

pub mod collector;
pub mod compress;
pub mod encrypt;
pub mod entry;
pub mod entry_errors;
pub mod sources;
pub mod tar;

use crate::config::BackupConfig;
use crate::error::{Context, Error, Result};
use entry_errors::EntryErrors;
use std::io::Write;
use tempfile::NamedTempFile;

/// A writer that can be finalized (flush compression state, write auth tags).
pub trait FinishableWrite: Write + Send {
    /// Consumes the writer and finalizes it. Must be called before drop to propagate errors.
    fn finish(self: Box<Self>) -> std::io::Result<()>;
}

/// Passthrough: plain writers (File, BufWriter) don't need finalization.
pub struct PassthroughWriter<W: Write + Send>(pub W);

impl<W: Write + Send> Write for PassthroughWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> { self.0.write(buf) }
    fn flush(&mut self) -> std::io::Result<()> { self.0.flush() }
}

impl<W: Write + Send> FinishableWrite for PassthroughWriter<W> {
    fn finish(mut self: Box<Self>) -> std::io::Result<()> { self.0.flush() }
}

/// Runs the full pipeline: collect entries, write tar → compress → encrypt → temp file.
/// Returns (temp_file, entry_errors).
pub fn run(config: &BackupConfig, pool: &rayon::ThreadPool) -> Result<(NamedTempFile, EntryErrors)> {
    let temp_file = match &config.temp_dir {
        Some(dir) => NamedTempFile::new_in(dir),
        None => NamedTempFile::new(),
    }
    .context("Failed to create temp file")?;
    let file = temp_file.reopen().context("Failed to reopen temp file")?;

    let (rx, entry_errors) = collector::collect(&config.files, pool, config.temp_dir.as_deref());

    // Build the writer chain: file ← encrypt ← compress ← tar
    let base_writer: Box<dyn FinishableWrite> = Box::new(PassthroughWriter(file));
    let encrypt_writer =
        encrypt::wrap_writer(&config.encryptor, base_writer).context("Failed to init encryptor")?;
    let compress_writer =
        compress::wrap_writer(&config.compressor, encrypt_writer).context("Failed to init compressor")?;

    // Tar writes into compress → encrypt → file
    let final_writer = tar::write_tar(rx, compress_writer)
        .map_err(|e| Error::from(e))
        .context("Tar creation failed")?;

    // Finalize: flush compression state + encryption auth tags
    final_writer.finish().context("Failed to finalize archive")?;

    Ok((temp_file, entry_errors))
}
