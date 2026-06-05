//! Tar archive creation from a channel of ArchiveEntry.

use crate::error::ArchiveError;
use crate::pipeline::entry::{ArchiveEntry, ArchiveEntryKind};
use crate::pipeline::FinishableWrite;
use std::sync::mpsc::Receiver;
use tar::{Builder, Header};

/// Consumes entries from the receiver and writes them as a tar archive.
/// Returns the inner writer for finalization.
pub fn write_tar(
    receiver: Receiver<ArchiveEntry>,
    writer: Box<dyn FinishableWrite>,
) -> std::result::Result<Box<dyn FinishableWrite>, ArchiveError> {
    let mut builder = Builder::new(writer);
    let mut count = 0;

    for entry in receiver {
        match entry.kind {
            ArchiveEntryKind::File(mut file) => {
                let size = file.metadata().map_err(ArchiveError::from)?.len();
                let mut header = Header::new_gnu();
                header.set_size(size);
                header.set_mode(0o644);
                header.set_entry_type(tar::EntryType::Regular);
                header.set_cksum();
                builder.append_data(&mut header, &entry.dst, &mut file)?;
            }
            ArchiveEntryKind::Memory(data) => {
                let mut header = Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                header.set_entry_type(tar::EntryType::Regular);
                header.set_cksum();
                builder.append_data(&mut header, &entry.dst, &*data)?;
            }
            ArchiveEntryKind::Symlink(target) => {
                let mut header = Header::new_gnu();
                header.set_size(0);
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_cksum();
                builder.append_link(&mut header, &entry.dst, &target)?;
            }
            ArchiveEntryKind::TempFile(mut file, temp_path) => {
                let size = file.metadata().map_err(ArchiveError::from)?.len();
                let mut header = Header::new_gnu();
                header.set_size(size);
                header.set_mode(0o644);
                header.set_entry_type(tar::EntryType::Regular);
                header.set_cksum();
                builder.append_data(&mut header, &entry.dst, &mut file)?;
                if let Err(e) = temp_path.close() {
                    tracing::warn!("Failed to remove temp file: {}", e);
                }
            }
        }
        count += 1;
    }

    tracing::info!("Processed {} archive entries", count);
    let writer = builder.into_inner()?;
    Ok(writer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::path::PathBuf;

    fn tar_to_vec(entries: Vec<ArchiveEntry>) -> Vec<u8> {
        use crate::pipeline::FinishableWrite;
        use std::io::Write;
        use std::sync::mpsc::sync_channel;
        use std::sync::{Arc, Mutex};

        struct SharedWriter(Arc<Mutex<Vec<u8>>>);
        impl Write for SharedWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().write(buf)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl FinishableWrite for SharedWriter {
            fn finish(self: Box<Self>) -> std::io::Result<()> {
                Ok(())
            }
        }

        let buf = Arc::new(Mutex::new(Vec::new()));
        let writer: Box<dyn FinishableWrite> = Box::new(SharedWriter(buf.clone()));

        let (tx, rx) = sync_channel(entries.len().max(1));
        for entry in entries {
            tx.send(entry).unwrap();
        }
        drop(tx);

        write_tar(rx, writer).unwrap().finish().unwrap();
        Arc::try_unwrap(buf).unwrap().into_inner().unwrap()
    }

    #[test]
    fn test_write_memory_entry() {
        let tar_data = tar_to_vec(vec![ArchiveEntry {
            dst: PathBuf::from("hello.txt"),
            kind: ArchiveEntryKind::Memory(b"hello world".to_vec().into()),
        }]);

        let mut archive = tar::Archive::new(tar_data.as_slice());
        for entry in archive.entries().unwrap() {
            let mut e = entry.unwrap();
            assert_eq!(e.path().unwrap().to_str().unwrap(), "hello.txt");
            let mut content = String::new();
            e.read_to_string(&mut content).unwrap();
            assert_eq!(content, "hello world");
        }
    }

    #[test]
    fn test_write_file_entry() {
        let mut builder = tar::Builder::new(Vec::new());
        let data = b"simulated file content";
        let mut header = Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder
            .append_data(&mut header, "from_file.txt", &data[..])
            .unwrap();
        let tar_data = builder.into_inner().unwrap();

        let mut archive = tar::Archive::new(tar_data.as_slice());
        for entry in archive.entries().unwrap() {
            let mut e = entry.unwrap();
            let mut content = String::new();
            e.read_to_string(&mut content).unwrap();
            assert_eq!(content, "simulated file content");
        }
    }

    #[test]
    fn test_write_symlink_entry() {
        let tar_data = tar_to_vec(vec![ArchiveEntry {
            dst: PathBuf::from("link"),
            kind: ArchiveEntryKind::Symlink(PathBuf::from("/target/path")),
        }]);

        let mut archive = tar::Archive::new(tar_data.as_slice());
        for entry in archive.entries().unwrap() {
            let e = entry.unwrap();
            assert_eq!(e.header().entry_type(), tar::EntryType::Symlink);
            assert_eq!(
                e.link_name().unwrap().unwrap().to_str().unwrap(),
                "/target/path"
            );
        }
    }

    #[test]
    fn test_empty_produces_valid_tar() {
        let tar_data = tar_to_vec(vec![]);
        let mut archive = tar::Archive::new(tar_data.as_slice());
        let entries: Vec<_> = archive.entries().unwrap().collect();
        assert_eq!(entries.len(), 0);
    }
}
