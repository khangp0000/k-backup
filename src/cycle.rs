//! Cycle outcome enum.

use std::path::PathBuf;

/// The result of a backup cycle.
pub enum CycleOutcome {
    /// Archive produced successfully (full or partial).
    Completed(PathBuf),
    /// Cycle was skipped (e.g., notification with on_failure:skip failed).
    Skipped(String),
}
