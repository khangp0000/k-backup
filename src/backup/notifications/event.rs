//! Backup event types for the notification system.

use crate::backup::backup_config::BackupConfig;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// Event types that notifications can subscribe to.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    BackupCycleStart,
    Success,
    NonFatalError,
    FatalError,
}

pub fn default_events() -> Vec<EventType> {
    vec![EventType::NonFatalError, EventType::FatalError]
}

/// A backup lifecycle event emitted during a backup cycle.
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
    pub fn event_type(&self) -> EventType {
        match self {
            Self::BackupCycleStart { .. } => EventType::BackupCycleStart,
            Self::Success { .. } => EventType::Success,
            Self::NonFatalError { .. } => EventType::NonFatalError,
            Self::FatalError { .. } => EventType::FatalError,
        }
    }
}
