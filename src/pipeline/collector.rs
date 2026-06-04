//! Entry collector using rayon fold/reduce with bounded channel.

use crate::config::ArchiveEntryConfig;
use crate::error::{ArchiveError, Error};
use crate::pipeline::entry::ArchiveEntry;
use crate::pipeline::entry_errors::{EntryError, EntryErrors};
use crate::pipeline::sources;
use rayon::prelude::*;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;

/// Runs the entry collector on the rayon pool.
/// Returns (entry_receiver, entry_errors) — receiver streams entries to tar writer.
pub fn collect(
    configs: &[ArchiveEntryConfig],
    pool: &rayon::ThreadPool,
    temp_dir: Option<&std::path::Path>,
) -> (Receiver<ArchiveEntry>, EntryErrors) {
    let channel_size = pool.current_num_threads().max(4);
    let (tx, rx) = sync_channel(channel_size);

    // Wrap each config in Arc for cheap cloning into errors
    let arced: Vec<Arc<ArchiveEntryConfig>> = configs.iter().map(|c| Arc::new(c.clone())).collect();
    let temp_dir_owned: Option<std::path::PathBuf> = temp_dir.map(|p| p.to_path_buf());

    let entry_errors = pool.install(|| {
        arced
            .par_iter()
            .fold(EntryErrors::new, |mut errors, config| {
                collect_one(config, &tx, &mut errors, temp_dir_owned.as_deref());
                errors
            })
            .reduce(EntryErrors::new, |mut a, b| {
                a.merge(b);
                a
            })
    });

    // Drop sender so receiver knows when all entries are sent
    drop(tx);

    (rx, entry_errors)
}

fn collect_one(
    config: &Arc<ArchiveEntryConfig>,
    tx: &SyncSender<ArchiveEntry>,
    errors: &mut EntryErrors,
    temp_dir: Option<&std::path::Path>,
) {
    match config.as_ref() {
        ArchiveEntryConfig::Sqlite(c) => match sources::sqlite::create_entry(c, temp_dir) {
            Ok(entry) => {
                let _ = tx.send(entry);
            }
            Err(e) => errors.push(EntryError {
                source: config.clone(),
                error: Error::from(e),
            }),
        },
        ArchiveEntryConfig::Base64(c) => match sources::base64::create_entry(c) {
            Ok(entry) => {
                let _ = tx.send(entry);
            }
            Err(e) => errors.push(EntryError {
                source: config.clone(),
                error: Error::from(e),
            }),
        },
        ArchiveEntryConfig::Glob(c) => match sources::glob::iter_entries(c) {
            Ok(results) => {
                for result in results {
                    match result {
                        Ok(entry) => {
                            let _ = tx.send(entry);
                        }
                        Err(e) => errors.push(EntryError {
                            source: config.clone(),
                            error: Error::from(e),
                        }),
                    }
                }
            }
            Err(e) => errors.push(EntryError {
                source: config.clone(),
                error: Error::from(e),
            }),
        },
    }
}
