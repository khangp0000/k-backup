//! Notification dispatch system.

pub mod command;
pub mod event;
pub mod smtp;

use crate::config::{BackupConfig, NotificationTarget, OnFailure};
use crate::error::Error;
use event::{BackupEvent, DispatchOutcome};

/// Dispatches an event to all subscribed notifications.
/// Sends to ALL, returns the highest-priority failure: Error > Skip > Continue (logged).
pub fn dispatch_event(config: &BackupConfig, event: &BackupEvent) -> DispatchOutcome {
    let event_type = event.event_type();
    let mut worst: Option<(OnFailure, Error)> = None;

    for (i, notif) in config.notifications.iter().enumerate() {
        if !notif.events.contains(&event_type) {
            continue;
        }

        let result = match &notif.target {
            NotificationTarget::Smtp(c) => smtp::send_event(c, event),
            NotificationTarget::Command(c) => command::send_event(c, event),
        };

        if let Err(e) = result {
            let name = notif.display_name(i);
            let e = e.context(format!("Notification '{}' failed", name));
            match notif.on_failure {
                OnFailure::Continue => {
                    tracing::error!("{} (continuing)", e);
                }
                OnFailure::Skip => {
                    tracing::error!("{} (skipping cycle)", e);
                    worst = Some(match worst {
                        Some((OnFailure::Error, existing)) => (OnFailure::Error, existing),
                        Some((OnFailure::Skip, existing)) => (OnFailure::Skip, existing),
                        _ => (OnFailure::Skip, e),
                    });
                }
                OnFailure::Error => {
                    tracing::error!("{} (fatal)", e);
                    worst = Some(match worst {
                        Some((OnFailure::Error, existing)) => {
                            (OnFailure::Error, Error::multiple(vec![existing, e]))
                        }
                        _ => (OnFailure::Error, e),
                    });
                }
            }
        }
    }

    match worst {
        None => DispatchOutcome::Ok,
        Some((OnFailure::Skip, e)) => DispatchOutcome::Skip(e),
        Some((OnFailure::Error, e)) => DispatchOutcome::Error(e),
        Some((OnFailure::Continue, _)) => unreachable!(),
    }
}
