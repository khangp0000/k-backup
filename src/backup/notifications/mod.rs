//! Event-based notification system for backup lifecycle.
//!
//! Supports multiple notification targets (SMTP, command) with per-target
//! event filtering and failure handling.

pub mod command;
pub mod event;
pub mod smtp;

use crate::backup::notifications::command::CommandNotificationConfig;
use crate::backup::notifications::event::{default_events, BackupEvent, EventType};
use crate::backup::notifications::smtp::SmtpNotificationConfig;
use crate::backup::result_error::result::Result;
use serde::{Deserialize, Serialize};
use std::result;
use validator::{Validate, ValidationErrors};

/// Wrapper struct for notification configuration.
///
/// Common fields (`name`, `events`, `on_failure`) are shared across all
/// notification types. The target-specific config is flattened via serde.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotificationConfig {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "default_events")]
    pub events: Vec<EventType>,
    #[serde(default)]
    pub on_failure: OnFailure,
    #[serde(flatten)]
    pub target: NotificationTarget,
}

/// Behavior when a notification fails to send.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OnFailure {
    /// Log and continue the current backup cycle (default).
    #[default]
    Continue,
    /// Abort current cycle, skip to next scheduled run.
    Skip,
    /// Abort and propagate error (stop the daemon).
    Error,
}

impl NotificationConfig {
    /// Returns the display name for logging. Falls back to "{type}-{index}".
    pub fn display_name(&self, index: usize) -> String {
        self.name.clone().unwrap_or_else(|| {
            let type_name = match &self.target {
                NotificationTarget::Smtp(_) => "smtp",
                NotificationTarget::Command(_) => "command",
            };
            format!("{}-{}", type_name, index)
        })
    }

    pub fn send_event(&self, event: &BackupEvent) -> Result<()> {
        self.target.send_event(event)
    }
}

impl Validate for NotificationConfig {
    fn validate(&self) -> result::Result<(), ValidationErrors> {
        match &self.target {
            NotificationTarget::Smtp(inner) => inner.validate(),
            NotificationTarget::Command(inner) => inner.validate(),
        }
    }
}

/// Target-specific notification configuration.
///
/// Note: cannot use deny_unknown_fields here because this enum is
/// deserialized via #[serde(flatten)] on the parent NotificationConfig.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationTarget {
    Smtp(SmtpNotificationConfig),
    Command(CommandNotificationConfig),
}

impl NotificationTarget {
    pub fn send_event(&self, event: &BackupEvent) -> Result<()> {
        match self {
            Self::Smtp(inner) => inner.send_event(event),
            Self::Command(inner) => inner.send_event(event),
        }
    }
}
