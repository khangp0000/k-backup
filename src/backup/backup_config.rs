use crate::backup::archive::{ArchiveEntry, ArchiveEntryConfig, ArchiveEntryIterable};
use crate::backup::compress::CompressorConfig;
use crate::backup::encrypt::EncryptorConfig;
use crate::backup::file_ext::FileExtProvider;
use crate::backup::tar;

use crate::backup::result_error::error::Error;
use crate::backup::result_error::result::convert_error_vec;
use crate::backup::result_error::result::Result;
use crate::backup::result_error::{AddDebugObjectAndFnName, AddMsg};
use crate::backup::retention::{ItemWithDateTime, RetentionConfig};
use chrono::{DateTime, TimeZone, Utc};
use itertools::Itertools;
use rayon::prelude::*;
use rayon::ThreadPool;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashSet;
use std::fmt::Display;
use std::fs::read_dir;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;
use tempfile::NamedTempFile;

use validator::{Validate, ValidationError};

/// Main configuration structure for the backup system
#[skip_serializing_none]
#[derive(Clone, Serialize, Deserialize, Debug, Validate)]
#[serde(deny_unknown_fields)]
pub struct BackupConfig {
    /// Cron expression defining backup schedule (UTC timezone)
    #[validate(custom(function = validate_cron_str))]
    pub cron: String,

    /// Base name for backup archive files
    #[validate(custom(function = validate_valid_archive_base_name))]
    pub archive_base_name: String,

    /// Directory where backup files will be created
    #[validate(custom(function = validate_out_dir))]
    pub out_dir: PathBuf,

    /// List of files and directories to include in backups
    pub files: Vec<ArchiveEntryConfig>,

    /// Compression configuration
    pub compressor: CompressorConfig,

    /// Encryption configuration
    pub encryptor: EncryptorConfig,

    /// Optional retention policy for automatic cleanup
    pub retention: Option<RetentionConfig>,
}

fn validate_cron_str(cron: &String) -> std::result::Result<(), ValidationError> {
    if cron_parser::parse(cron, &Utc::now()).is_err() {
        return Err(ValidationError::new("InvalidCron")
            .with_message(format!("Invalid cron string: {cron:?}").into()));
    }

    Ok(())
}

fn validate_out_dir(dir: &PathBuf) -> std::result::Result<(), ValidationError> {
    if dir.exists() {
        if !dir.is_dir() {
            return Err(ValidationError::new("InvalidDirectory")
                .with_message("out_dir is not a directory".into()));
        }
    } else {
        return std::fs::create_dir_all(dir).map_err(|e| {
            ValidationError::new("InvalidDirectory").with_message(
                format!("cannot create or access out_dir path {:?}: {}", dir, e).into(),
            )
        });
    }

    Ok(())
}

#[cfg(target_family = "unix")]
fn validate_valid_archive_base_name(name: &str) -> std::result::Result<(), ValidationError> {
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

#[cfg(target_os = "windows")]
fn validate_valid_archive_base_name(_name: &str) -> std::result::Result<(), ValidationError> {
    Ok(())
}

static TIME_FORMAT: &str = "%Y-%m-%dT%Hh%Mm%Ss%z";

impl FileExtProvider for BackupConfig {
    fn file_ext(&self) -> Option<impl AsRef<str>> {
        Some(
            std::iter::once("tar")
                .chain(self.compressor.file_ext().iter().map(|s| s.as_ref()))
                .chain(self.encryptor.file_ext().iter().map(|s| s.as_ref()))
                .collect::<Vec<_>>()
                .join("."),
        )
    }
}

impl BackupConfig {
    /// Generates timestamp-based filename extension
    fn time_file_ext<O: Display, T: TimeZone<Offset = O>>(&self, dt: DateTime<T>) -> String {
        let time_str = dt.format(TIME_FORMAT).to_string().replace('+', "_");
        match self.file_ext() {
            Some(ext) => format!("{}.{}", time_str, ext.as_ref() as &str),
            None => time_str,
        }
    }

    /// Extracts timestamp from backup filename
    ///
    /// Returns None if filename doesn't match expected format
    pub fn get_date_time_from_file_path<P: AsRef<Path>>(
        &self,
        file_path: P,
    ) -> Option<DateTime<Utc>> {
        let file_name = file_path.as_ref().file_name()?.to_str()?;
        let (start_idx, end_idx) = match self.file_ext() {
            Some(ext) => {
                let end = format!(".{}", ext.as_ref() as &str);
                if !file_name.ends_with(&end) {
                    return None;
                }
                let start = format!("{}.", self.archive_base_name);
                if !file_name.starts_with(&start) {
                    return None;
                }
                let start_idx = start.len();
                let end_idx = file_name.len() - end.len();
                (start_idx, end_idx)
            }
            None => {
                if !file_name.starts_with(&format!("{}.", self.archive_base_name)) {
                    return None;
                }
                let start_idx = self.archive_base_name.len() + 1;
                let end_idx = file_name.len();
                (start_idx, end_idx)
            }
        };

        if end_idx < start_idx {
            return None;
        }

        let time_string = file_name[start_idx..end_idx].replace('_', "+");

        DateTime::parse_from_str(&time_string, TIME_FORMAT)
            .ok()
            .map(|dt| dt.to_utc())
    }

    /// Spawns background thread to collect archive entries
    ///
    /// Returns (thread_handle, entry_receiver)
    fn spawn_entry_collector(
        &self,
        pre_process_pool: Arc<ThreadPool>,
    ) -> (JoinHandle<Result<()>>, Receiver<Result<ArchiveEntry>>) {
        let (result_tx, result_rx) = sync_channel(pre_process_pool.current_num_threads());
        let config_clone = self.clone();
        let handle = std::thread::spawn(move || {
            convert_error_vec(pre_process_pool.install(|| {
                config_clone
                    .files
                    .par_iter()
                    .map(|archive_entry_config| {
                        archive_entry_config.archive_entry_iterator().map(|iter| {
                            let errors = iter
                                .filter_map(|archive_entry_result| {
                                    archive_entry_result
                                        .add_msg("Ignoring entry")
                                        .and_then(|archive_entry| {
                                            result_tx.send(Ok(archive_entry)).map_err(Error::from)
                                        })
                                        .err()
                                })
                                .collect_vec();
                            convert_error_vec(errors)
                        })
                    })
                    .filter_map(|res| match res {
                        Ok(r) => r.err(),
                        Err(e) => result_tx.send(Err(e)).map_err(Error::from).err(),
                    })
                    .collect()
            }))
        });
        (handle, result_rx)
    }

    /// Creates backup archive with compression and encryption
    ///
    /// Returns (archive_path, non_fatal_error)
    pub fn create_archive(
        &self,
        dt: DateTime<Utc>,
        pre_process_pool: Arc<ThreadPool>,
    ) -> Result<(PathBuf, Option<Error>)> {
        tracing::info!(
            "Creating archive with {} worker threads",
            pre_process_pool.current_num_threads()
        );

        let (entry_handle, entry_rx) = self.spawn_entry_collector(pre_process_pool);

        let file_name = format!("{}.{}", self.archive_base_name, self.time_file_ext(dt),);
        tracing::info!("Creating archive file: {}", file_name);
        let config_clone = self.clone();

        let archive_handle = std::thread::spawn(move || -> Result<NamedTempFile> {
            tar::create_tar_and_process(entry_rx, &config_clone.encryptor, &config_clone.compressor)
        });

        let archive_create_res = match archive_handle.join().unwrap() {
            Ok(temp_file) => {
                let file_path = self.out_dir.join(file_name);
                tracing::info!("Finalizing archive: moving from temp to final location");
                temp_file
                    .persist(&file_path)
                    .map(|_| 0)
                    .or_else(|e| std::fs::copy(e.file, &file_path))
                    .map(|_| file_path)
                    .map_err(Error::from)
            }
            Err(e) => Err(e),
        }
        .add_debug_object_and_fn_name(self.clone(), "create_write_archive");

        let entry_create_res = entry_handle.join().unwrap();
        match archive_create_res {
            Ok(fp) => Ok((fp, entry_create_res.err())),
            Err(e1) => match entry_create_res {
                Ok(_) => Err(e1),
                Err(e2) => Err(e1.chain(e2)),
            },
        }
    }

    /// Executes one backup cycle: retention cleanup and backup creation
    ///
    /// Returns next scheduled backup time
    pub fn execute_backup_cycle(
        &self,
        backup_set: &mut HashSet<Rc<ItemWithDateTime<PathBuf, Utc>>>,
        now: DateTime<Utc>,
        pre_process_pool: Arc<ThreadPool>,
    ) -> Result<DateTime<Utc>> {
        if let Some(retention) = &self.retention {
            let backups_to_delete = retention
                .get_delete(backup_set.iter(), now)
                .into_iter()
                .cloned()
                .collect_vec();
            if !backups_to_delete.is_empty() {
                tracing::info!(
                    "Retention cleanup: removing {} expired backups",
                    backups_to_delete.len()
                );
            }
            backups_to_delete.into_iter().for_each(|to_delete| {
                tracing::info!("Removing expired backup: {:?}", &to_delete.item);
                let removed = backup_set.remove(&to_delete);
                if !removed {
                    panic!("Remove item in memory {:?} failed", &to_delete.item);
                }
                let _ = std::fs::remove_file(&to_delete.item);
            });
        }

        tracing::info!(
            "Starting backup creation for {} file sources",
            self.files.len()
        );
        let (file_path, non_fatal_error) = self.create_archive(now, pre_process_pool)?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
        tracing::info!(
            "Successfully created backup: {:?} ({} bytes)",
            &file_path,
            file_size
        );

        if let Some(non_fatal_error) = non_fatal_error {
            tracing::warn!("Non-fatal error during backup: {non_fatal_error}");
        }

        backup_set.insert(Rc::new(ItemWithDateTime::from((file_path, now))));
        tracing::info!("Total backups now: {}", backup_set.len());

        let next_backup = cron_parser::parse(&self.cron, &now).unwrap();
        tracing::info!("Next backup scheduled for: {}", next_backup);

        Ok(next_backup)
    }

    /// Main daemon loop that runs backups on schedule
    pub fn start_loop(&self, pre_process_pool: Arc<ThreadPool>) -> Result<()> {
        tracing::info!("Starting backup daemon with cron schedule: {}", self.cron);
        tracing::info!("Backup output directory: {:?}", self.out_dir);
        tracing::info!("Archive base name: {}", self.archive_base_name);

        let mut set: HashSet<_> = read_dir(&self.out_dir)?
            .filter_map(|r| r.ok())
            .filter_map(|r| {
                self.get_date_time_from_file_path(r.path())
                    .map(|dt| ItemWithDateTime::from((r.path(), dt)))
            })
            .map(Rc::new)
            .collect();

        tracing::info!("Found {} existing backup files", set.len());

        let start = set
            .iter()
            .map(|i| i.date_time.clone())
            .sorted_unstable()
            .next_back()
            .unwrap_or(DateTime::UNIX_EPOCH.to_utc().into());

        let cron = &self.cron;
        let mut start = cron_parser::parse(cron, start.as_ref()).unwrap();

        loop {
            let now = Utc::now();

            if now < start {
                tracing::info!("Sleeping until {start}");
                std::thread::sleep((start - now).to_std().unwrap())
            } else {
                start = self.execute_backup_cycle(&mut set, now, pre_process_pool.clone())?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::compress::CompressorConfig;
    use crate::backup::encrypt::EncryptorConfig;
    use crate::backup::retention::RetentionConfig;
    use chrono::{TimeZone, Utc};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_test_config() -> BackupConfig {
        let temp_dir = TempDir::new().unwrap();
        BackupConfig {
            cron: "0 1 * * *".to_string(),
            archive_base_name: "test_backup".to_string(),
            out_dir: temp_dir.path().to_path_buf(),
            files: vec![],
            compressor: CompressorConfig::None,
            encryptor: EncryptorConfig::None,
            retention: None,
        }
    }

    #[test]
    fn test_valid_cron_expressions() {
        let valid_crons = vec!["0 1 * * *", "0 */6 * * *", "0 2 * * 0", "*/15 * * * *"];

        for cron in valid_crons {
            let result = validate_cron_str(&cron.to_string());
            assert!(result.is_ok(), "Cron '{}' should be valid", cron);
        }
    }

    #[test]
    fn test_invalid_cron_expressions() {
        let invalid_crons = vec!["invalid", "60 1 * * *", "0 25 * * *", "0 1 32 * *"];

        for cron in invalid_crons {
            let result = validate_cron_str(&cron.to_string());
            assert!(result.is_err(), "Cron '{}' should be invalid", cron);
        }
    }

    #[test]
    fn test_file_ext_generation() {
        let config = create_test_config();
        let ext = config.file_ext();
        assert!(ext.is_some());
        assert_eq!(ext.unwrap().as_ref() as &str, "tar");
    }

    #[test]
    fn test_time_file_ext() {
        let config = create_test_config();
        let dt = Utc.with_ymd_and_hms(2024, 1, 15, 14, 30, 45).unwrap();
        let ext = config.time_file_ext(dt);
        assert_eq!(ext, "2024-01-15T14h30m45s_0000.tar");
    }

    #[test]
    fn test_get_date_time_from_file_path() {
        let config = create_test_config();
        let dt = Utc.with_ymd_and_hms(2024, 1, 15, 14, 30, 45).unwrap();

        let filename = format!("test_backup.{}", config.time_file_ext(dt));
        let path = PathBuf::from(&filename);

        let parsed_dt = config.get_date_time_from_file_path(&path);
        assert!(parsed_dt.is_some());
        assert_eq!(parsed_dt.unwrap().date_naive(), dt.date_naive());
    }

    #[test]
    fn test_get_date_time_from_invalid_path() {
        let config = create_test_config();

        let invalid_paths = vec![
            "wrong_prefix.2024-01-15T14h30m45s_0000.tar",
            "test_backup.invalid_timestamp.tar",
            "test_backup.2024-01-15T14h30m45s_0000.wrong_ext",
        ];

        for path in invalid_paths {
            let result = config.get_date_time_from_file_path(PathBuf::from(path));
            assert!(result.is_none(), "Path '{}' should not parse", path);
        }
    }

    #[test]
    #[cfg(target_family = "unix")]
    fn test_archive_base_name_validation() {
        let valid_names = vec!["backup", "test_backup", "my-backup-123"];
        for name in valid_names {
            let result = validate_valid_archive_base_name(name);
            assert!(result.is_ok(), "Name '{}' should be valid", name);
        }

        let long_name = "x".repeat(101);
        let invalid_names = vec!["backup/with/slash", "backup\0with\0null", &long_name];
        for name in invalid_names {
            let result = validate_valid_archive_base_name(name);
            assert!(result.is_err(), "Name '{}' should be invalid", name);
        }
    }

    #[test]
    fn test_out_dir_validation() {
        let temp_dir = TempDir::new().unwrap();

        let result = validate_out_dir(&temp_dir.path().to_path_buf());
        assert!(result.is_ok());

        let new_dir = temp_dir.path().join("new_dir");
        let result = validate_out_dir(&new_dir.clone());
        assert!(result.is_ok());
        assert!(new_dir.exists());
    }

    #[test]
    fn test_backup_config_invalid_cron() {
        let temp_dir = TempDir::new().unwrap();

        let config = BackupConfig {
            cron: "invalid cron".to_string(),
            archive_base_name: "test".to_string(),
            out_dir: temp_dir.path().to_path_buf(),
            files: vec![],
            compressor: CompressorConfig::None,
            encryptor: EncryptorConfig::None,
            retention: None,
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_execute_backup_cycle() {
        use crate::backup::archive::base64::Base64Source;

        use rayon::ThreadPoolBuilder;
        use std::fs::{create_dir_all, write};
        use std::time::Duration as StdDuration;

        let temp_dir = TempDir::new().unwrap();
        let backup_dir = temp_dir.path().join("backup");
        create_dir_all(&backup_dir).unwrap();

        let config = BackupConfig {
            cron: "0 1 * * *".to_string(),
            archive_base_name: "test_backup".to_string(),
            out_dir: backup_dir.clone(),
            files: vec![ArchiveEntryConfig::Base64(Base64Source::new(
                "test content".as_bytes().into(),
                PathBuf::from("test.txt"),
            ))],
            compressor: CompressorConfig::None,
            encryptor: EncryptorConfig::None,
            retention: Some(RetentionConfig {
                default_retention: StdDuration::from_secs(2 * 24 * 3600), // 2 days
                daily_retention: None,
                monthly_retention: None,
                yearly_retention: None,
                min_backups: 1,
            }),
        };

        let pool = Arc::new(ThreadPoolBuilder::new().num_threads(1).build().unwrap());
        let mut backup_set = HashSet::new();
        let now = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();

        // Create some existing backup files to test retention
        let old_backup_path = backup_dir.join(format!(
            "test_backup.{}.tar",
            (now - chrono::Duration::days(5)).format("%Y-%m-%dT%Hh%Mm%Ss_0000")
        ));
        let recent_backup_path = backup_dir.join(format!(
            "test_backup.{}.tar",
            (now - chrono::Duration::hours(12)).format("%Y-%m-%dT%Hh%Mm%Ss_0000")
        ));

        // Create the actual files
        write(&old_backup_path, "old backup content").unwrap();
        write(&recent_backup_path, "recent backup content").unwrap();

        // Add them to backup set
        backup_set.insert(Rc::new(ItemWithDateTime::from((
            old_backup_path.clone(),
            now - chrono::Duration::days(5),
        ))));
        backup_set.insert(Rc::new(ItemWithDateTime::from((
            recent_backup_path.clone(),
            now - chrono::Duration::hours(12),
        ))));

        assert_eq!(backup_set.len(), 2);
        assert!(old_backup_path.exists());
        assert!(recent_backup_path.exists());

        let next_backup = config
            .execute_backup_cycle(&mut backup_set, now, pool)
            .unwrap();

        // Verify retention cleanup worked correctly
        assert!(!old_backup_path.exists(), "Old backup should be deleted");
        assert!(recent_backup_path.exists(), "Recent backup should be kept");

        // Verify new backup was created and added to set
        assert_eq!(backup_set.len(), 2); // recent backup + new backup
        let new_backup = backup_set
            .iter()
            .find(|item| *item.date_time == now)
            .expect("New backup should be in set");
        assert!(new_backup.item.exists(), "New backup file should exist");

        // Verify next backup time is in the future
        assert!(next_backup > now);
    }

    #[test]
    fn test_execute_backup_cycle_with_complex_retention() {
        use crate::backup::archive::base64::Base64Source;

        use rayon::ThreadPoolBuilder;
        use std::fs::{create_dir_all, write};
        use std::time::Duration as StdDuration;

        let temp_dir = TempDir::new().unwrap();
        let backup_dir = temp_dir.path().join("backup");
        create_dir_all(&backup_dir).unwrap();

        let config = BackupConfig {
            cron: "0 1 * * *".to_string(),
            archive_base_name: "test_backup".to_string(),
            out_dir: backup_dir.clone(),
            files: vec![ArchiveEntryConfig::Base64(Base64Source::new(
                "test content".as_bytes().into(),
                PathBuf::from("test.txt"),
            ))],
            compressor: CompressorConfig::None,
            encryptor: EncryptorConfig::None,
            retention: Some(RetentionConfig {
                default_retention: StdDuration::from_secs(3 * 24 * 3600), // 3 days
                daily_retention: Some(StdDuration::from_secs(7 * 24 * 3600)), // 7 days
                monthly_retention: None,
                yearly_retention: None,
                min_backups: 2,
            }),
        };

        let pool = Arc::new(ThreadPoolBuilder::new().num_threads(1).build().unwrap());
        let mut backup_set = HashSet::new();
        let now = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();

        // Create test backup files
        let very_old_path = backup_dir.join(format!(
            "test_backup.{}.tar",
            (now - chrono::Duration::days(10)).format("%Y-%m-%dT%Hh%Mm%Ss_0000")
        ));
        let old_but_daily_kept_path = backup_dir.join(format!(
            "test_backup.{}.tar",
            (now - chrono::Duration::days(5)).format("%Y-%m-%dT%Hh%Mm%Ss_0000")
        ));
        let recent_path = backup_dir.join(format!(
            "test_backup.{}.tar",
            (now - chrono::Duration::hours(12)).format("%Y-%m-%dT%Hh%Mm%Ss_0000")
        ));

        // Create the actual files
        write(&very_old_path, "very old backup").unwrap();
        write(&old_but_daily_kept_path, "old but daily kept backup").unwrap();
        write(&recent_path, "recent backup").unwrap();

        // Add them to backup set
        backup_set.insert(Rc::new(ItemWithDateTime::from((
            very_old_path.clone(),
            now - chrono::Duration::days(10),
        ))));
        backup_set.insert(Rc::new(ItemWithDateTime::from((
            old_but_daily_kept_path.clone(),
            now - chrono::Duration::days(5),
        ))));
        backup_set.insert(Rc::new(ItemWithDateTime::from((
            recent_path.clone(),
            now - chrono::Duration::hours(12),
        ))));

        assert_eq!(backup_set.len(), 3);
        assert!(very_old_path.exists());
        assert!(old_but_daily_kept_path.exists());
        assert!(recent_path.exists());

        let next_backup = config
            .execute_backup_cycle(&mut backup_set, now, pool)
            .unwrap();

        // Verify retention cleanup worked correctly
        assert!(
            !very_old_path.exists(),
            "Very old backup should be deleted (outside daily retention)"
        );
        assert!(
            old_but_daily_kept_path.exists(),
            "Old backup should be kept (within daily retention)"
        );
        assert!(
            recent_path.exists(),
            "Recent backup should be kept (within default retention)"
        );

        // Verify new backup was created
        assert_eq!(backup_set.len(), 3); // old_but_daily_kept + recent + new backup
        let new_backup = backup_set
            .iter()
            .find(|item| *item.date_time == now)
            .expect("New backup should be in set");
        assert!(new_backup.item.exists(), "New backup file should exist");

        // Verify next backup time is in the future
        assert!(next_backup > now);
    }

    #[test]
    fn test_execute_backup_cycle_min_backups_safety() {
        use crate::backup::archive::base64::Base64Source;

        use rayon::ThreadPoolBuilder;
        use std::fs::{create_dir_all, write};
        use std::time::Duration as StdDuration;

        let temp_dir = TempDir::new().unwrap();
        let backup_dir = temp_dir.path().join("backup");
        create_dir_all(&backup_dir).unwrap();

        let config = BackupConfig {
            cron: "0 1 * * *".to_string(),
            archive_base_name: "test_backup".to_string(),
            out_dir: backup_dir.clone(),
            files: vec![ArchiveEntryConfig::Base64(Base64Source::new(
                "test content".as_bytes().into(),
                PathBuf::from("test.txt"),
            ))],
            compressor: CompressorConfig::None,
            encryptor: EncryptorConfig::None,
            retention: Some(RetentionConfig {
                default_retention: StdDuration::from_secs(1), // 1 second (very short)
                daily_retention: None,
                monthly_retention: None,
                yearly_retention: None,
                min_backups: 3, // Safety net
            }),
        };

        let pool = Arc::new(ThreadPoolBuilder::new().num_threads(1).build().unwrap());
        let mut backup_set = HashSet::new();
        let now = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();

        // Create 2 old backup files (all should be expired by default_retention)
        let old_backup1_path = backup_dir.join(format!(
            "test_backup.{}.tar",
            (now - chrono::Duration::days(1)).format("%Y-%m-%dT%Hh%Mm%Ss_0000")
        ));
        let old_backup2_path = backup_dir.join(format!(
            "test_backup.{}.tar",
            (now - chrono::Duration::days(2)).format("%Y-%m-%dT%Hh%Mm%Ss_0000")
        ));

        write(&old_backup1_path, "old backup 1").unwrap();
        write(&old_backup2_path, "old backup 2").unwrap();

        backup_set.insert(Rc::new(ItemWithDateTime::from((
            old_backup1_path.clone(),
            now - chrono::Duration::days(1),
        ))));
        backup_set.insert(Rc::new(ItemWithDateTime::from((
            old_backup2_path.clone(),
            now - chrono::Duration::days(2),
        ))));

        assert_eq!(backup_set.len(), 2);
        assert!(old_backup1_path.exists());
        assert!(old_backup2_path.exists());

        let next_backup = config
            .execute_backup_cycle(&mut backup_set, now, pool)
            .unwrap();

        // Both old backups should be kept due to min_backups safety net
        // even though they're expired by default_retention
        assert!(
            old_backup1_path.exists(),
            "Old backup 1 should be kept (min_backups safety)"
        );
        assert!(
            old_backup2_path.exists(),
            "Old backup 2 should be kept (min_backups safety)"
        );

        // New backup should be created
        assert_eq!(backup_set.len(), 3); // 2 old + 1 new = 3 (exactly min_backups)
        let new_backup = backup_set
            .iter()
            .find(|item| *item.date_time == now)
            .expect("New backup should be in set");
        assert!(new_backup.item.exists(), "New backup file should exist");

        assert!(next_backup > now);
    }

    #[test]
    fn test_create_archive_with_compression_and_encryption() {
        use crate::backup::archive::base64::Base64Source;
        use crate::backup::archive::walkdir_globset::{
            CustomDeserializedGlob, WalkdirAndGlobsetSource,
        };
        use crate::backup::compress::xz::XzConfig;
        use crate::backup::encrypt::age::{AgeEncryptorConfig, RedactedString};
        use ::tar::Archive;
        use age::Decryptor;

        use liblzma::read::XzDecoder;
        use rayon::ThreadPoolBuilder;

        use std::fs::{create_dir_all, write};
        use std::io::{BufReader, Read};

        let temp_dir = TempDir::new().unwrap();
        let source_dir = temp_dir.path().join("source");
        let backup_dir = temp_dir.path().join("backup");

        // Create test files for glob source
        create_dir_all(&source_dir).unwrap();
        create_dir_all(&backup_dir).unwrap();
        write(source_dir.join("test.txt"), "file content").unwrap();

        let txt_glob: CustomDeserializedGlob = serde_json::from_str("\"**/*.txt\"").unwrap();
        let passphrase = "test-passphrase-123";

        let config = BackupConfig {
            cron: "0 1 * * *".to_string(),
            archive_base_name: "test_backup".to_string(),
            out_dir: backup_dir.clone(),
            files: vec![
                ArchiveEntryConfig::Base64(Base64Source::new(
                    "base64 content".as_bytes().into(),
                    PathBuf::from("base64.txt"),
                )),
                ArchiveEntryConfig::Glob(WalkdirAndGlobsetSource::new(
                    source_dir.clone(),
                    None::<PathBuf>,
                    Some(vec![txt_glob]),
                )),
            ],
            compressor: CompressorConfig::Xz(XzConfig::new(6, Some(2)).unwrap()),
            encryptor: EncryptorConfig::Age(AgeEncryptorConfig::Passphrase {
                passphrase: RedactedString::from(passphrase),
            }),
            retention: None,
        };

        let pool = Arc::new(ThreadPoolBuilder::new().num_threads(2).build().unwrap());
        let dt = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();

        // Create encrypted and compressed archive
        let (archive_path, error) = config.create_archive(dt, pool).unwrap();
        assert!(error.is_none());
        assert!(archive_path.exists());

        // Verify file extension
        let filename = archive_path.file_name().unwrap().to_str().unwrap();
        assert!(filename.ends_with(".tar.xz.age"));

        // Decrypt and decompress to verify content
        let encrypted_file = std::fs::File::open(&archive_path).unwrap();
        let decryptor = Decryptor::new(BufReader::new(encrypted_file)).unwrap();

        let identity = age::scrypt::Identity::new(age::secrecy::SecretString::new(
            passphrase.to_string().into(),
        ));
        let decrypted_reader = decryptor
            .decrypt(std::iter::once(&identity as &dyn age::Identity))
            .unwrap();
        let mut decompressed_reader = XzDecoder::new(BufReader::new(decrypted_reader));

        // Read TAR content
        let mut archive = Archive::new(&mut decompressed_reader);
        let entries = archive.entries().unwrap();
        let mut found_content = Vec::new();

        for entry in entries {
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap().to_string_lossy().to_string();
            let mut content = String::new();
            entry.read_to_string(&mut content).unwrap();
            found_content.push((path, content));
        }

        // Sort for consistent comparison
        found_content.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(found_content.len(), 2);
        assert_eq!(
            found_content[0],
            ("base64.txt".to_string(), "base64 content".to_string())
        );
        assert_eq!(
            found_content[1],
            ("test.txt".to_string(), "file content".to_string())
        );
    }
}
