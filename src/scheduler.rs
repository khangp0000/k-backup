//! Backup scheduler: pre-validate, pipeline, post-validate, persist, retention, dispatch.

use crate::config::{ArchiveEntryConfig, BackupConfig};
use crate::cycle::CycleOutcome;
use crate::error::{Context, Error, Result};
use crate::notifications;
use crate::notifications::event::{BackupEvent, DispatchOutcome};
use crate::pipeline;
use crate::retention::{self, BackupFile};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

static TIME_FORMAT: &str = "%Y-%m-%dT%Hh%Mm%Ss%z";

/// Runs the scheduler in daemon mode (cron loop).
pub fn start_loop(config: Arc<BackupConfig>, pool: Arc<rayon::ThreadPool>) -> Result<()> {
    let cron = config.cron.as_ref().ok_or_else(|| {
        Error::from(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "cron field is required for daemon mode",
        ))
    })?;

    tracing::info!("Starting backup daemon with cron: {}", cron);
    tracing::info!("Output directory: {:?}", config.out_dir);

    let mut backup_set = scan_existing_backups(&config)?;
    tracing::info!("Found {} existing backup files", backup_set.len());

    let cron_str = cron.as_str();
    let mut next = compute_initial_start(&backup_set, cron_str);

    loop {
        let now = Utc::now();
        if now < next {
            tracing::info!("Sleeping until {}", next);
            std::thread::sleep((next - now).to_std().unwrap());
        } else {
            match execute_cycle(&config, &pool, &mut backup_set, now) {
                Ok(CycleOutcome::Completed(_)) | Ok(CycleOutcome::Skipped(_)) => {}
                Err(e) => return Err(e),
            }
            next = cron_parser::parse(cron_str, &now).unwrap();
            tracing::info!("Next backup scheduled for: {}", next);
        }
    }
}

/// Runs a single backup cycle and exits.
pub fn run_once(config: Arc<BackupConfig>, pool: Arc<rayon::ThreadPool>) -> Result<()> {
    tracing::info!("Running single backup cycle");
    tracing::info!("Output directory: {:?}", config.out_dir);

    let mut backup_set = scan_existing_backups(&config)?;
    tracing::info!("Found {} existing backup files", backup_set.len());

    let now = Utc::now();
    match execute_cycle(&config, &pool, &mut backup_set, now)? {
        CycleOutcome::Completed(p) => {
            tracing::info!("Backup completed: {:?}", p);
        }
        CycleOutcome::Skipped(reason) => {
            tracing::warn!("Backup cycle skipped: {}", reason);
        }
    }
    Ok(())
}

/// Executes one backup cycle.
fn execute_cycle(
    config: &Arc<BackupConfig>,
    pool: &Arc<rayon::ThreadPool>,
    backup_set: &mut HashMap<Rc<Path>, DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<CycleOutcome> {
    // 1. Dispatch start event
    match notifications::dispatch_event(
        config,
        &BackupEvent::BackupCycleStart {
            config: config.clone(),
            timestamp: now,
        },
    ) {
        DispatchOutcome::Ok => {}
        DispatchOutcome::Skip(e) => return Ok(CycleOutcome::Skipped(e.to_string())),
        DispatchOutcome::Error(e) => return Err(e),
    }

    // 2. Pre-validate required sources
    pre_validate(config)?;

    // 3. Run pipeline
    let (temp_file, entry_errors) = pipeline::run(config, pool)?;

    // 4. Post-validate: check required failures
    if entry_errors.has_required_failure() {
        let err_msg = entry_errors.to_string();
        // Dispatch fatal event
        let _ = notifications::dispatch_event(
            config,
            &BackupEvent::FatalError {
                config: config.clone(),
                timestamp: now,
                error: err_msg.clone(),
            },
        );
        return Err(Error::from(std::io::Error::other(format!(
            "Required source(s) failed:\n{}",
            err_msg
        ))));
    }

    // 5. Persist temp → final path
    let file_name = format!(
        "{}.{}.{}",
        config.archive_base_name,
        now.format(TIME_FORMAT).to_string().replace('+', "_"),
        config.file_ext(),
    );
    let final_path = Rc::from(config.out_dir.join(&file_name));
    match temp_file.persist_noclobber(&final_path) {
        Ok(_) => {}
        Err(e) => {
            if final_path.exists() {
                return Err(Error::from(std::io::Error::from(e))
                    .context("Failed to persist archive: target already exists"));
            }
            tracing::info!("Rename failed ({}), falling back to copy", e.error);
            std::fs::copy(e.file.path(), &final_path)
                .map_err(Error::from)
                .context("Failed to persist archive (copy fallback)")?;
        }
    }

    let file_size = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
    tracing::info!("Created backup: {:?} ({} bytes)", final_path, file_size);

    // 6. Update backup set
    let final_rc: Rc<Path> = Rc::clone(&final_path);
    backup_set.insert(Rc::clone(&final_rc), now);

    // 7. Retention
    if let Some(ref retention_config) = config.retention {
        let backups: Vec<BackupFile> = backup_set
            .iter()
            .map(|(p, t)| BackupFile {
                path: Rc::clone(p),
                timestamp: *t,
            })
            .collect();
        let backup_refs: Vec<&BackupFile> = backups.iter().collect();
        let to_delete = retention::get_deletions(&backup_refs, now, retention_config);
        if !to_delete.is_empty() {
            tracing::info!("Retention: removing {} expired backups", to_delete.len());
        }
        for path in &to_delete {
            tracing::info!("Deleting: {:?}", path);
            if let Err(e) = std::fs::remove_file(path.as_ref()) {
                tracing::error!("Failed to delete {:?}: {}", path, e);
            } else {
                backup_set.remove(path.as_ref());
            }
        }
    }

    // 8. Dispatch success/non-fatal event
    let outcome = if !entry_errors.is_empty() {
        tracing::warn!("Non-fatal errors:\n{}", entry_errors);
        let dispatch = notifications::dispatch_event(
            config,
            &BackupEvent::NonFatalError {
                config: config.clone(),
                timestamp: now,
                output_file: final_path.to_path_buf(),
                errors: entry_errors.to_string(),
            },
        );
        match dispatch {
            DispatchOutcome::Ok => {}
            DispatchOutcome::Skip(e) => return Ok(CycleOutcome::Skipped(e.to_string())),
            DispatchOutcome::Error(e) => return Err(e),
        }
        CycleOutcome::Completed(final_path.to_path_buf())
    } else {
        let dispatch = notifications::dispatch_event(
            config,
            &BackupEvent::Success {
                config: config.clone(),
                timestamp: now,
                output_file: final_path.to_path_buf(),
            },
        );
        match dispatch {
            DispatchOutcome::Ok => {}
            DispatchOutcome::Skip(e) => return Ok(CycleOutcome::Skipped(e.to_string())),
            DispatchOutcome::Error(e) => return Err(e),
        }
        CycleOutcome::Completed(final_path.to_path_buf())
    };

    tracing::info!("Total backups: {}", backup_set.len());
    Ok(outcome)
}

/// Pre-validates required sources.
fn pre_validate(config: &BackupConfig) -> Result<()> {
    use crate::pipeline::sources;

    for entry in &config.files {
        match entry {
            ArchiveEntryConfig::Sqlite(c) if c.required => {
                sources::sqlite::validate(c)
                    .context(format!("Pre-validation failed for {:?}", c.src))?;
            }
            ArchiveEntryConfig::Glob(c) if c.required => {
                sources::glob::validate(c)
                    .context(format!("Pre-validation failed for {:?}", c.src_dir))?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Scans existing backup files in out_dir.
fn scan_existing_backups(config: &BackupConfig) -> Result<HashMap<Rc<Path>, DateTime<Utc>>> {
    let mut map = HashMap::new();
    let entries = std::fs::read_dir(&config.out_dir).context("Failed to read out_dir")?;

    let ext = config.file_ext();
    let prefix = format!("{}.", config.archive_base_name);

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to read entry in out_dir: {}", e);
                continue;
            }
        };
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with(&prefix) && name.ends_with(&format!(".{}", ext)) {
                let time_part = &name[prefix.len()..name.len() - ext.len() - 1];
                let time_str = time_part.replace('_', "+");
                if let Ok(dt) = DateTime::parse_from_str(&time_str, TIME_FORMAT) {
                    let rc: Rc<Path> = path.into();
                    map.insert(rc, dt.to_utc());
                }
            }
        }
    }
    Ok(map)
}

/// Computes the initial start time for the cron loop.
fn compute_initial_start(
    backup_set: &HashMap<Rc<Path>, DateTime<Utc>>,
    cron_str: &str,
) -> DateTime<Utc> {
    backup_set
        .values()
        .max()
        .copied()
        .map(|last| cron_parser::parse(cron_str, &last).unwrap())
        .unwrap_or_else(|| cron_parser::parse(cron_str, &DateTime::UNIX_EPOCH).unwrap())
}
