use crate::backup::archive::{ArchiveEntryConfig, ArchiveEntryIterable};
use crate::backup::compress::{CompressorBuilder, CompressorConfig};
use crate::backup::encrypt::{EncryptorBuilder, EncryptorConfig};
use crate::backup::file_ext::FileExtProvider;
use crate::backup::finish::Finish;
use crate::backup::result_error::error::Error;
use crate::backup::result_error::result::convert_error_vec;
use crate::backup::result_error::result::Result;
use crate::backup::result_error::{WithDebugObjectAndFnName, WithMsg};
use crate::backup::retention::{ItemWithDateTime, RetentionConfig};
use chrono::{DateTime, TimeZone, Utc};
use itertools::Itertools;
use rayon::prelude::*;
use rayon::ThreadPool;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashSet;
use std::fmt::Display;
use std::fs::{read_dir, File};
use std::io::{BufWriter, IntoInnerError};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, OnceLock};
use tracing::{info, warn};
use validator::{Validate, ValidationError};

#[skip_serializing_none]
#[derive(Clone, Serialize, Deserialize, Debug, Validate)]
pub struct BackupConfig {
    #[validate(custom(function = validate_cron_str))]
    pub cron: Arc<str>,
    #[validate(custom(function = validate_valid_archive_base_name))]
    pub archive_base_name: Arc<str>,
    #[validate(custom(function = validate_out_dir))]
    pub out_dir: Arc<Path>,
    pub files: Arc<Vec<ArchiveEntryConfig>>,
    pub compressor: Arc<CompressorConfig>,
    pub encryptor: Arc<EncryptorConfig>,
    pub retention: Option<Arc<RetentionConfig>>,
}

fn validate_cron_str(cron: &Arc<str>) -> std::result::Result<(), ValidationError> {
    if cron_parser::parse(cron.as_ref(), &Utc::now()).is_err() {
        return Err(ValidationError::new("InvalidCron")
            .with_message(format!("Invalid cron string: {cron:?}").into()));
    }

    Ok(())
}

fn validate_out_dir(dir: &Arc<Path>) -> std::result::Result<(), ValidationError> {
    if dir.exists() {
        if !dir.is_dir() {
            return Err(ValidationError::new("InvalidDirectory")
                .with_message("out_dir is not a directory".into()));
        }
    } else {
        return std::fs::create_dir_all(&dir).map_err(|e| {
            ValidationError::new("InvalidDirectory").with_message(
                format!("cannot create or access out_dir path {:?}: {}", dir, e).into(),
            )
        });
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_valid_archive_base_name(name: &Arc<str>) -> std::result::Result<(), ValidationError> {
    if name.chars().any(|c| c == '/' || c == '\0') {
        return Err(ValidationError::new("InvalidArchiveBaseName")
            .with_message("Invalid archive_base_name, must not contain '/' or null".into()));
    }

    if name.len() > 100 {
        return Err(ValidationError::new("InvalidArchiveBaseName")
            .with_message("Invalid archive_base_name, maximum len is 100".into()));
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn validate_valid_archive_base_name(name: &Arc<str>) -> Result<(), ValidationError> {
    Ok(())
}

#[cfg(target_os = "windows")]
fn validate_valid_archive_base_name(name: &Arc<str>) -> Result<(), ValidationError> {
    Ok(())
}

static TIME_FORMAT: &str = "%Y-%m-%dT%Hh%Mm%Ss%z";
static TAR_FILE_EXT: OnceLock<Arc<str>> = OnceLock::new();

impl FileExtProvider for BackupConfig {
    fn file_ext(&self) -> Option<Arc<str>> {
        Some(
            std::iter::once(TAR_FILE_EXT.get_or_init(|| "tar".into()))
                .chain(self.compressor.file_ext().iter())
                .chain(self.encryptor.file_ext().iter())
                .join(".")
                .into(),
        )
    }
}

impl BackupConfig {
    fn time_file_ext<O: Display, T: TimeZone<Offset = O>>(&self, dt: DateTime<T>) -> Arc<str> {
        format!(
            "{}.{}",
            dt.format(TIME_FORMAT).to_string().replace('+', "_"),
            self.file_ext().unwrap_or("".into())
        )
        .into()
    }

    pub fn get_date_time_from_file_path<P: AsRef<Path>>(
        &self,
        file_path: P,
    ) -> Option<DateTime<Utc>> {
        let file_name = file_path.as_ref().file_name()?.to_str()?;
        let end = format!(".{}", self.file_ext().unwrap_or("".into()));
        if !file_name.ends_with(end.as_str()) {
            return None;
        }
        let start = format!("{}.", self.archive_base_name);
        if !file_name.starts_with(start.as_str()) {
            return None;
        }

        let start_idx = start.len();
        let end_idx = file_name.len() - end.len();
        if end_idx < start_idx {
            return None;
        }

        let time_string = file_name[start_idx..end_idx].replace('_', "+");

        DateTime::parse_from_str(time_string.as_str(), TIME_FORMAT)
            .ok()
            .map(|dt| dt.to_utc())
    }

    pub fn create_archive(
        &self,
        dt: DateTime<Utc>,
        pre_process_pool: Arc<ThreadPool>,
    ) -> Result<(PathBuf, Option<Error>)> {
        let (result_tx, result_rx) = sync_channel(pre_process_pool.current_num_threads());
        let config_clone = self.clone();
        let entry_create_join_handle = std::thread::spawn(move || {
            convert_error_vec(pre_process_pool.install(|| {
                let i = config_clone
                    .files
                    .as_ref()
                    .par_iter()
                    .map(|archive_entry_config| {
                        archive_entry_config.archive_entry_iterator().map(|iter| {
                            let errors = iter
                                .filter_map(|archive_entry_result| {
                                    archive_entry_result
                                        .with_msg("Ignoring entry")
                                        .and_then(|archive_entry| {
                                            result_tx.send(Ok(archive_entry)).map_err(Error::from)
                                        })
                                        .err()
                                })
                                .collect_vec();
                            return convert_error_vec(errors);
                        })
                    })
                    .filter_map(|res| match res {
                        Ok(r) => r.err(),
                        Err(e) => result_tx.send(Err(e)).map_err(Error::from).err(),
                    })
                    .collect();
                i
            }))
        });

        let config_clone = self.clone();
        let file_name = format!(
            "{}.{}",
            config_clone.archive_base_name,
            config_clone.time_file_ext(dt),
        );
        let file_path_tmp = Arc::new(config_clone.out_dir.join(format!("{file_name}.tmp")));
        let file_path_tmp_clone = file_path_tmp.clone();
        let archive_file_join_handle = std::thread::spawn(move || -> Result<_> {
            let mut writer = File::create_new(file_path_tmp_clone.as_path())
                .map(BufWriter::new)
                .map_err(Error::from)
                .and_then(|f| config_clone.encryptor.build_encryptor(f))
                .map(BufWriter::new)
                .and_then(|f| config_clone.compressor.build_compressor(f))
                .map(BufWriter::new)
                .map(|f| tar::Builder::new(f))?;

            writer.follow_symlinks(true);

            for entry in result_rx {
                let entry = entry?;
                writer.append_path_with_name(&entry.src, &entry.dst)?;
                if entry.delete_src {
                    std::fs::remove_file(entry.src)?
                }
            }

            writer
                .into_inner()?
                .into_inner()
                .map_err(IntoInnerError::into_error)?
                .finish()?
                .into_inner()
                .map_err(IntoInnerError::into_error)?
                .finish()?
                .into_inner()
                .map_err(IntoInnerError::into_error)?;

            Ok(())
        });

        let archive_create_res = match archive_file_join_handle.join().unwrap() {
            Ok(_) => {
                let file_path = config_clone.out_dir.join(file_name);
                std::fs::rename(file_path_tmp.as_path(), &file_path)
                    .map(|_| file_path)
                    .map_err(|e| Error::from(e))
            }
            Err(e) => Err(e.with_debug_object_and_fn_name(self.clone(), "create_write_archive")),
        }
        .map_err(|mut e| {
            if let Err(e2) = std::fs::remove_file(file_path_tmp.as_path()) {
                e = e.chain(e2.into())
            }

            return e.with_msg("Delete tmp file failed.");
        });

        let entry_create_res = entry_create_join_handle.join().unwrap();
        match archive_create_res {
            Ok(fp) => Ok((fp, entry_create_res.err())),
            Err(e1) => match entry_create_res {
                Ok(_) => Err(e1),
                Err(e2) => Err(e1.chain(e2)),
            },
        }
    }

    pub fn start_loop(&self, pre_process_pool: Arc<ThreadPool>) -> Result<()> {
        let mut set: HashSet<_> = read_dir(&self.out_dir)?
            .into_iter()
            .filter_map(|r| r.ok())
            .filter_map(|r| {
                self.get_date_time_from_file_path(&r.path())
                    .map(|dt| ItemWithDateTime::from((r.path(), dt)))
            })
            .map(Rc::new)
            .collect();

        let start = set
            .iter()
            .map(|i| i.date_time.clone())
            .sorted_unstable()
            .last()
            .unwrap_or(DateTime::UNIX_EPOCH.to_utc().into());
        let cron = self.cron.as_ref();
        let mut start = cron_parser::parse(cron, start.as_ref()).unwrap();
        loop {
            let now = Utc::now();
            if now < start {
                info!("Sleeping until {start}");
                std::thread::sleep((start - now).to_std().unwrap())
            } else {
                if let Some(retention) = &self.retention {
                    retention
                        .get_delete(set.iter().cloned(), now)
                        .for_each(|to_delete| {
                            info!("Removing out of retention file {:?}", &to_delete.item);
                            let removed = set.remove(&to_delete);
                            if !removed {
                                panic!("Remove item in memory {:?} failed", &to_delete.item);
                            }
                            let _ = std::fs::remove_file(&to_delete.item);
                        });
                }
                info!("Trying to create backup...");

                let (file_path, non_fatal_error) =
                    self.create_archive(now, pre_process_pool.clone())?;
                info!("Created backup file: {:?}", &file_path);
                if let Some(non_fatal_error) = non_fatal_error {
                    warn!("Received non fatal error: {non_fatal_error}")
                }
                set.insert(Rc::new(ItemWithDateTime::from((file_path, now))));
                start = cron_parser::parse(cron, &now).unwrap();
            }
        }
    }
}
