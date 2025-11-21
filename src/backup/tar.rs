use crate::backup::archive::{ArchiveEntry, ArchiveSource};
use crate::backup::compress::{CompressorBuilder, CompressorConfig};
use crate::backup::encrypt::{EncryptorBuilder, EncryptorConfig};
use crate::backup::finish::Finish;
use crate::backup::result_error::result::Result;
use std::io::{BufWriter, IntoInnerError, Seek};
use std::sync::mpsc::Receiver;
use tempfile::NamedTempFile;

/// Creates TAR archive from entries
///
/// Returns seekable temporary file containing the TAR archive
pub fn create_tar_archive(entry_rx: Receiver<Result<ArchiveEntry>>) -> Result<NamedTempFile> {
    let mut writer = tar::Builder::new(NamedTempFile::new()?);
    writer.follow_symlinks(true);

    let mut entry_count = 0;
    for entry in entry_rx {
        let mut entry = entry?;
        match &mut entry.src {
            ArchiveSource::Path(path) => {
                writer.append_path_with_name(path.as_ref(), entry.dst.as_ref())?;
            }
            ArchiveSource::Reader(reader) => {
                let mut header = tar::Header::new_gnu();
                let mut tar_writer = writer.append_writer(&mut header, entry.dst.as_ref())?;
                std::io::copy(reader.as_mut(), &mut tar_writer)?;
                // tar_writer automatically calls finish() when dropped
            }
        }
        entry_count += 1;
    }
    tracing::info!("Processed {} archive entries", entry_count);
    let mut tar_temp = writer.into_inner()?;

    tar_temp.seek(std::io::SeekFrom::Start(0))?;
    Ok(tar_temp)
}

/// Creates TAR archive and processes through compression/encryption pipeline
///
/// Returns temporary file containing the final processed archive
pub fn create_tar_and_process(
    entry_rx: Receiver<Result<ArchiveEntry>>,
    encryptor: &EncryptorConfig,
    compressor: &CompressorConfig,
) -> Result<NamedTempFile> {
    let tar_temp = create_tar_archive(entry_rx)?;
    let mut final_temp = NamedTempFile::new()?;

    let mut final_writer = encryptor
        .build_encryptor(BufWriter::new(&mut final_temp))
        .map(BufWriter::new)
        .and_then(|f| compressor.build_compressor(f))
        .map(BufWriter::new)?;

    std::io::copy(&mut tar_temp.into_file(), &mut final_writer)?;

    final_writer
        .into_inner()
        .map_err(IntoInnerError::into_error)?
        .finish()?
        .into_inner()
        .map_err(IntoInnerError::into_error)?
        .finish()?
        .into_inner()
        .map_err(IntoInnerError::into_error)?;

    Ok(final_temp)
}
