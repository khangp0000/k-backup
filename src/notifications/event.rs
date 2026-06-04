//! Backup event types and dispatch outcome.

use crate::config::BackupConfig;
use crate::error::Error;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;

/// A backup lifecycle event.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackupEvent {
    BackupCycleStart {
        config: Arc<BackupConfig>,
        timestamp: DateTime<Utc>,
    },
    Success {
        config: Arc<BackupConfig>,
        timestamp: DateTime<Utc>,
        output_file: PathBuf,
    },
    NonFatalError {
        config: Arc<BackupConfig>,
        timestamp: DateTime<Utc>,
        output_file: PathBuf,
        errors: String,
    },
    FatalError {
        config: Arc<BackupConfig>,
        timestamp: DateTime<Utc>,
        error: String,
    },
}

impl BackupEvent {
    pub fn event_type(&self) -> crate::config::EventType {
        use crate::config::EventType;
        match self {
            Self::BackupCycleStart { .. } => EventType::BackupCycleStart,
            Self::Success { .. } => EventType::Success,
            Self::NonFatalError { .. } => EventType::NonFatalError,
            Self::FatalError { .. } => EventType::FatalError,
        }
    }
}

/// Result of dispatching an event to all subscribed notifications.
pub enum DispatchOutcome {
    Ok,
    Skip(Error),
    Error(Error),
}
