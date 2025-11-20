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

use validator::{Validate, ValidationError};

/// Main configuration structure for the backup system
/// 
/// This struct defines all aspects of backup behavior including:
/// - When backups run (cron schedule)
/// - What gets backed up (files/directories)
/// - How backups are processed (compression, encryption)
/// - Where backups are stored and how they're named
/// - How long backups are retained
#[skip_serializing_none]
#[derive(Clone, Serialize, Deserialize, Debug, Validate)]
#[serde(deny_unknown_fields)]
pub struct BackupConfig {
    /// Cron expression defining backup schedule
    /// 
    /// Examples:
    /// - "0 1 * * *" = Daily at 1:00 AM UTC
    /// - "0 */6 * * *" = Every 6 hours
    /// - "0 2 * * 0" = Weekly on Sunday at 2:00 AM UTC
    #[validate(custom(function = validate_cron_str))]
    pub cron: Arc<str>,
    
    /// Base name for backup archive files
    /// 
    /// Final filename format: {archive_base_name}.{timestamp}.{extensions}
    /// Example: "backup" → "backup.2025-11-20T15h39m11s+0000.tar.xz.age"
    #[validate(custom(function = validate_valid_archive_base_name))]
    pub archive_base_name: Arc<str>,
    
    /// Directory where backup files will be created
    /// 
    /// Must be writable by the backup process.
    /// Used for both temporary files during creation and final backup storage.
    #[validate(custom(function = validate_out_dir))]
    pub out_dir: Arc<Path>,
    
    /// List of files and directories to include in backups
    /// 
    /// Supports multiple source types:
    /// - SQLite databases (with proper backup API)
    /// - File/directory patterns (with glob matching)
    pub files: Arc<Vec<ArchiveEntryConfig>>,
    
    /// Compression configuration
    /// 
    /// Defines how backup archives are compressed before encryption.
    /// Currently supports XZ (LZMA) compression with configurable levels.
    pub compressor: Arc<CompressorConfig>,
    
    /// Encryption configuration
    /// 
    /// Defines how backup archives are encrypted after compression.
    /// Currently supports Age encryption with passphrase or key files.
    pub encryptor: Arc<EncryptorConfig>,
    
    /// Optional retention policy for automatic cleanup
    /// 
    /// If specified, old backups are automatically deleted based on:
    /// - Default retention period for all backups
    /// - Special retention for daily/monthly/yearly backups
    /// If None, no automatic cleanup is performed.
    pub retention: Option<Arc<RetentionConfig>>,
}

/// Validates that the cron expression is syntactically correct
/// 
/// Uses the cron_parser crate to verify the expression can be parsed
/// and will generate valid future timestamps.
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

/// Timestamp format used in backup filenames
/// 
/// Format: YYYY-MM-DDTHH:MM:SS+TIMEZONE
/// Example: "2025-11-20T15h39m11s+0000"
/// 
/// The '+' in timezone is replaced with '_' to avoid filesystem issues
/// on systems that don't handle '+' well in filenames.
static TIME_FORMAT: &str = "%Y-%m-%dT%Hh%Mm%Ss%z";

/// Cached file extension for TAR archives
/// Built dynamically based on compression and encryption settings
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
    /// Generates a timestamp-based file extension for backup files
    /// 
    /// Creates a string like "2025-11-20T15h39m11s_0000.tar.xz.age"
    /// The '+' in timezone offset is replaced with '_' for filesystem compatibility.
    fn time_file_ext<O: Display, T: TimeZone<Offset = O>>(&self, dt: DateTime<T>) -> Arc<str> {
        format!(
            "{}.{}",
            dt.format(TIME_FORMAT).to_string().replace('+', "_"),
            self.file_ext().unwrap_or("".into())
        )
        .into()
    }

    /// Extracts timestamp from a backup filename
    /// 
    /// Parses filenames created by time_file_ext() back into DateTime objects.
    /// Used for retention management to determine backup age.
    /// 
    /// Returns None if the filename doesn't match expected format.
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
        tracing::info!("Creating archive with {} worker threads", pre_process_pool.current_num_threads());
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
        tracing::info!("Creating archive file: {}", file_name);
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

            let mut entry_count = 0;
            for entry in result_rx {
                let entry = entry?;
                writer.append_path_with_name(&entry.src, &entry.dst)?;
                if entry.delete_src {
                    std::fs::remove_file(entry.src)?
                }
                entry_count += 1;
            }
            tracing::info!("Processed {} archive entries", entry_count);

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
                tracing::info!("Finalizing archive: moving from temp to final location");
                std::fs::rename(file_path_tmp.as_path(), &file_path)
                    .map(|_| file_path)
                    .map_err(|e| Error::from(e))
            }
            Err(e) => Err(e.with_debug_object_and_fn_name(self.clone(), "create_write_archive")),
        }
        .map_err(|mut e| {
            if let Err(e2) = std::fs::remove_file(file_path_tmp.as_path()) {
                e = e.chain(e2.into()).with_msg("Delete tmp file failed.")
            }

            return e;
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

    /// Main daemon loop that runs backups on schedule
    /// 
    /// This function:
    /// 1. Scans existing backup files to build retention state
    /// 2. Calculates next backup time based on cron schedule
    /// 3. Sleeps until backup time arrives
    /// 4. Runs retention cleanup (deletes old backups)
    /// 5. Creates new backup
    /// 6. Repeats forever
    /// 
    /// The loop never exits normally - any error causes the process to terminate.
    /// Uses the provided thread pool for parallel operations during backup creation.
    pub fn start_loop(&self, pre_process_pool: Arc<ThreadPool>) -> Result<()> {
        tracing::info!("Starting backup daemon with cron schedule: {}", self.cron);
        tracing::info!("Backup output directory: {:?}", self.out_dir);
        tracing::info!("Archive base name: {}", self.archive_base_name);
        
        // Build initial set of existing backup files for retention management
        // Each file is parsed to extract its creation timestamp
        let mut set: HashSet<_> = read_dir(&self.out_dir)?
            .into_iter()
            .filter_map(|r| r.ok())
            .filter_map(|r| {
                self.get_date_time_from_file_path(&r.path())
                    .map(|dt| ItemWithDateTime::from((r.path(), dt)))
            })
            .map(Rc::new)
            .collect();
        
        tracing::info!("Found {} existing backup files", set.len());

        // Find the most recent backup timestamp to calculate next backup time
        // If no backups exist, start from Unix epoch
        let start = set
            .iter()
            .map(|i| i.date_time.clone())
            .sorted_unstable()
            .last()
            .unwrap_or(DateTime::UNIX_EPOCH.to_utc().into());
            
        let cron = self.cron.as_ref();
        // Calculate next backup time based on cron schedule
        let mut start = cron_parser::parse(cron, start.as_ref()).unwrap();
        
        loop {
            let now = Utc::now();
            
            // Sleep until it's time for the next backup
            if now < start {
                tracing::info!("Sleeping until {start}");
                std::thread::sleep((start - now).to_std().unwrap())
            } else {
                // Run retention cleanup before creating new backup
                if let Some(retention) = &self.retention {
                    let backups_to_delete = retention.get_delete(set.iter().cloned(), now);
                    if !backups_to_delete.is_empty() {
                        tracing::info!("Retention cleanup: removing {} expired backups", backups_to_delete.len());
                    }
                    backups_to_delete
                        .into_iter()
                        .for_each(|to_delete| {
                            tracing::info!("Removing expired backup: {:?}", &to_delete.item);
                            let removed = set.remove(&to_delete);
                            if !removed {
                                // FIXME: This panic will crash the daemon
                                // Should log error and continue instead
                                panic!("Remove item in memory {:?} failed", &to_delete.item);
                            }
                            // FIXME: Ignoring file deletion errors could lead to disk space issues
                            let _ = std::fs::remove_file(&to_delete.item);
                        });
                }
                
                tracing::info!("Starting backup creation for {} file sources", self.files.len());
                // Create the actual backup archive
                let (file_path, non_fatal_error) =
                    self.create_archive(now, pre_process_pool.clone())?;
                
                let file_size = std::fs::metadata(&file_path)
                    .map(|m| m.len())
                    .unwrap_or(0);
                tracing::info!("Successfully created backup: {:?} ({} bytes)", &file_path, file_size);
                
                if let Some(non_fatal_error) = non_fatal_error {
                    tracing::warn!("Non-fatal error during backup: {non_fatal_error}");
                }
                
                // Add new backup to retention tracking
                set.insert(Rc::new(ItemWithDateTime::from((file_path, now))));
                tracing::info!("Total backups now: {}", set.len());
                
                // Calculate next backup time
                start = cron_parser::parse(cron, &now).unwrap();
                tracing::info!("Next backup scheduled for: {}", start);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::compress::CompressorConfig;
    use crate::backup::encrypt::EncryptorConfig;
    use chrono::{TimeZone, Utc};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_test_config() -> BackupConfig {
        let temp_dir = TempDir::new().unwrap();
        BackupConfig {
            cron: "0 1 * * *".into(),
            archive_base_name: "test_backup".into(),
            out_dir: temp_dir.path().into(),
            files: Arc::new(vec![]),
            compressor: Arc::new(CompressorConfig::None),
            encryptor: Arc::new(EncryptorConfig::None),
            retention: None,
        }
    }

    #[test]
    fn test_valid_cron_expressions() {
        let valid_crons = vec![
            "0 1 * * *",     // Daily at 1 AM
            "0 */6 * * *",   // Every 6 hours
            "0 2 * * 0",     // Weekly on Sunday
            "*/15 * * * *",  // Every 15 minutes
        ];

        for cron in valid_crons {
            let result = validate_cron_str(&cron.into());
            assert!(result.is_ok(), "Cron '{}' should be valid", cron);
        }
    }

    #[test]
    fn test_invalid_cron_expressions() {
        let invalid_crons = vec![
            "invalid",
            "60 1 * * *",    // Invalid minute
            "0 25 * * *",    // Invalid hour
            "0 1 32 * *",    // Invalid day
        ];

        for cron in invalid_crons {
            let result = validate_cron_str(&cron.into());
            assert!(result.is_err(), "Cron '{}' should be invalid", cron);
        }
    }

    #[test]
    fn test_file_ext_generation() {
        let config = create_test_config();
        let ext = config.file_ext();
        assert_eq!(ext, Some("tar".into()));
    }

    #[test]
    fn test_time_file_ext() {
        let config = create_test_config();
        let dt = Utc.with_ymd_and_hms(2025, 1, 15, 14, 30, 45).unwrap();
        let ext = config.time_file_ext(dt);
        assert!(ext.contains("2025-01-15T14h30m45s"));
        assert!(ext.contains("tar"));
    }

    #[test]
    fn test_get_date_time_from_file_path() {
        let config = create_test_config();
        let dt = Utc.with_ymd_and_hms(2025, 1, 15, 14, 30, 45).unwrap();
        
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
            "wrong_prefix.2025-01-15T14h30m45s_0000.tar",
            "test_backup.invalid_timestamp.tar",
            "test_backup.2025-01-15T14h30m45s_0000.wrong_ext",
        ];
        
        for path in invalid_paths {
            let result = config.get_date_time_from_file_path(PathBuf::from(path));
            assert!(result.is_none(), "Path '{}' should not parse", path);
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_archive_base_name_validation() {
        // Valid names
        let valid_names = vec!["backup", "test_backup", "my-backup-123"];
        for name in valid_names {
            let result = validate_valid_archive_base_name(&name.into());
            assert!(result.is_ok(), "Name '{}' should be valid", name);
        }

        // Invalid names
        let long_name = "x".repeat(101);
        let invalid_names = vec![
            "backup/with/slash",
            "backup\0with\0null",
            &long_name, // Too long
        ];
        for name in invalid_names {
            let result = validate_valid_archive_base_name(&name.into());
            assert!(result.is_err(), "Name '{}' should be invalid", name);
        }
    }

    #[test]
    fn test_out_dir_validation() {
        let temp_dir = TempDir::new().unwrap();
        
        // Valid directory
        let result = validate_out_dir(&temp_dir.path().into());
        assert!(result.is_ok());
        
        // Non-existent directory (should be created)
        let new_dir = temp_dir.path().join("new_dir");
        let result = validate_out_dir(&new_dir.clone().into());
        assert!(result.is_ok());
        assert!(new_dir.exists());
    }

    #[test]
    fn test_backup_config_validation() {
        let temp_dir = TempDir::new().unwrap();
        
        let config = BackupConfig {
            cron: "0 1 * * *".into(),
            archive_base_name: "test".into(),
            out_dir: temp_dir.path().into(),
            files: Arc::new(vec![]),
            compressor: Arc::new(CompressorConfig::None),
            encryptor: Arc::new(EncryptorConfig::None),
            retention: None,
        };
        
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_backup_config_invalid_cron() {
        let temp_dir = TempDir::new().unwrap();
        
        let config = BackupConfig {
            cron: "invalid cron".into(),
            archive_base_name: "test".into(),
            out_dir: temp_dir.path().into(),
            files: Arc::new(vec![]),
            compressor: Arc::new(CompressorConfig::None),
            encryptor: Arc::new(EncryptorConfig::None),
            retention: None,
        };
        
        assert!(config.validate().is_err());
    }
}