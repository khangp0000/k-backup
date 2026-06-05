//! Entry collector using rayon fold/reduce with bounded channel.

use crate::config::ArchiveEntryConfig;
use crate::error::Error;
use crate::pipeline::entry::ArchiveEntry;
use crate::pipeline::entry_errors::{EntryError, EntryErrors};
use crate::pipeline::sources;
use rayon::prelude::*;
use std::ops::ControlFlow;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;

/// Spawns entry collection on a background thread.
/// Returns (entry_receiver, join_handle) — receiver streams entries to tar writer,
/// join handle resolves to entry errors when collection completes.
pub fn collect(
    configs: &[ArchiveEntryConfig],
    pool: &Arc<rayon::ThreadPool>,
    temp_dir: Option<&std::path::Path>,
) -> (Receiver<ArchiveEntry>, JoinHandle<EntryErrors>) {
    let channel_size = pool.current_num_threads().max(4);
    let (tx, rx) = sync_channel(channel_size);

    let arced: Vec<Arc<ArchiveEntryConfig>> = configs.iter().map(|c| Arc::new(c.clone())).collect();
    let temp_dir_owned: Option<std::path::PathBuf> = temp_dir.map(|p| p.to_path_buf());
    let pool = Arc::clone(pool);

    let handle = std::thread::spawn(move || {
        let errors = pool.install(|| collect_all(&arced, &tx, temp_dir_owned.as_deref()));
        drop(tx);
        errors
    });

    (rx, handle)
}

/// Core collection logic. Processes all configs, sending entries through tx.
/// Short-circuits if the channel closes.
fn collect_all(
    configs: &[Arc<ArchiveEntryConfig>],
    tx: &SyncSender<ArchiveEntry>,
    temp_dir: Option<&std::path::Path>,
) -> EntryErrors {
    let result = configs
        .par_iter()
        .try_fold(EntryErrors::new, |mut errors, config| {
            collect_one(config, tx, &mut errors, temp_dir)
        })
        .try_reduce(EntryErrors::new, |mut a, b| {
            a.merge(b);
            ControlFlow::Continue(a)
        });
    match result {
        ControlFlow::Continue(errors) | ControlFlow::Break(errors) => errors,
    }
}

fn collect_one(
    config: &Arc<ArchiveEntryConfig>,
    tx: &SyncSender<ArchiveEntry>,
    errors: &mut EntryErrors,
    temp_dir: Option<&std::path::Path>,
) -> ControlFlow<EntryErrors, EntryErrors> {
    match config.as_ref() {
        ArchiveEntryConfig::Sqlite(c) => match sources::sqlite::create_entry(c, temp_dir) {
            Ok(entry) => {
                if let Err(_) = tx.send(entry) {
                    errors.push(EntryError {
                        source: config.clone(),
                        error: Error::from(std::io::Error::other("archive writer closed")),
                    });
                    return ControlFlow::Break(std::mem::take(errors));
                }
            }
            Err(e) => errors.push(EntryError {
                source: config.clone(),
                error: Error::from(e),
            }),
        },
        ArchiveEntryConfig::Base64(c) => {
            let entry = sources::base64::create_entry(c);
            if let Err(_) = tx.send(entry) {
                errors.push(EntryError {
                    source: config.clone(),
                    error: Error::from(std::io::Error::other("archive writer closed")),
                });
                return ControlFlow::Break(std::mem::take(errors));
            }
        }
        ArchiveEntryConfig::Glob(c) => {
            let errs = match sources::glob::send_entries(c, tx) {
                std::ops::ControlFlow::Continue(errs) => errs,
                std::ops::ControlFlow::Break(errs) => {
                    for e in errs {
                        errors.push(EntryError {
                            source: config.clone(),
                            error: Error::from(e),
                        });
                    }
                    errors.push(EntryError {
                        source: config.clone(),
                        error: Error::from(std::io::Error::other("archive writer closed")),
                    });
                    return ControlFlow::Break(std::mem::take(errors));
                }
            };
            for e in errs {
                errors.push(EntryError {
                    source: config.clone(),
                    error: Error::from(e),
                });
            }
        }
    }
    ControlFlow::Continue(std::mem::take(errors))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ArchiveEntryConfig, Base64SourceConfig};

    #[test]
    fn short_circuits_when_channel_closed() {
        let configs: Vec<Arc<ArchiveEntryConfig>> = (0..100)
            .map(|i| {
                Arc::new(ArchiveEntryConfig::Base64(Base64SourceConfig {
                    content: crate::config::Base64Bytes::new(b"a".to_vec()),
                    dst: format!("{}.txt", i).into(),
                }))
            })
            .collect();

        // Create channel and immediately drop receiver — channel is closed
        let (tx, _rx) = std::sync::mpsc::sync_channel(4);
        drop(_rx);

        let errors = collect_all(&configs, &tx, None);

        // Should have exactly 1 error: "archive writer closed"
        assert_eq!(errors.errors.len(), 1);
        assert!(errors.errors[0]
            .error
            .to_string()
            .contains("archive writer closed"));
    }
}
