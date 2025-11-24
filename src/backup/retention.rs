use bon::Builder;
use getset::Getters;
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::fmt::Debug;
use validator::Validate;

fn default_min_backups() -> usize {
    3
}

/// Configuration for backup retention policies
///
/// Implements grandfather-father-son backup rotation with configurable retention periods:
/// - `default_retention`: Base retention applied to all backups
/// - `daily_retention`: Keeps one backup per day for specified duration
/// - `monthly_retention`: Keeps one backup per month for specified duration  
/// - `yearly_retention`: Keeps one backup per year for specified duration
/// - `min_backups`: Safety net - minimum backups to always keep regardless of age
///
/// The algorithm preserves the most recent backup in each time category,
/// allowing for sophisticated backup rotation schemes while preventing
/// accidental deletion of all backups.
#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, Deserialize, Validate, Builder, Getters)]
#[serde(deny_unknown_fields)]
#[getset(get = "pub")]
pub struct RetentionConfig {
    /// Base retention period applied to all backups
    ///
    /// Backups older than this duration are eligible for deletion,
    /// unless they're preserved by daily/monthly/yearly retention rules.
    #[serde(with = "humantime_serde")]
    default_retention: std::time::Duration,

    /// How long to keep daily backups (one per day)
    ///
    /// The most recent backup from each day within this period is preserved.
    /// Example: "7days" keeps one backup per day for the last week.
    #[serde(with = "humantime_serde")]
    daily_retention: Option<std::time::Duration>,

    /// How long to keep monthly backups (one per month)
    ///
    /// The most recent backup from each month within this period is preserved.
    /// Example: "3months" keeps one backup per month for the last 3 months.
    #[serde(with = "humantime_serde")]
    monthly_retention: Option<std::time::Duration>,

    /// How long to keep yearly backups (one per year)
    ///
    /// The most recent backup from each year within this period is preserved.
    /// Example: "5years" keeps one backup per year for the last 5 years.
    #[serde(with = "humantime_serde")]
    yearly_retention: Option<std::time::Duration>,

    /// Minimum number of backups to always keep
    ///
    /// Safety net to prevent all backups from being deleted if the system
    /// hasn't run for a long time. Always keeps at least this many of the
    /// most recent backups, regardless of age.
    #[serde(default = "default_min_backups")]
    #[builder(default = default_min_backups())]
    min_backups: usize,
}



impl Default for RetentionConfig {
    fn default() -> Self {
        Self::builder()
            .default_retention(std::time::Duration::from_secs(0))
            .build()
    }
}

impl RetentionConfig {
    /// Determines which backups should be deleted based on retention policy
    ///
    /// Implements grandfather-father-son backup rotation:
    /// 1. Applies default retention to all backups
    /// 2. Preserves the most recent backup from each day/month/year
    /// 3. Ensures at least min_backups are always kept (safety net)
    ///
    /// Returns list of backups that should be deleted
    pub fn get_delete<R, T, I, II>(&self, iter: I, now: DateTime<Utc>) -> Vec<II>
    where
        T: TimeZone,
        II: AsRef<ItemWithDateTime<R, T>>,
        I: IntoIterator<Item = II>,
    {
        let default_retention = Duration::from_std(self.default_retention).unwrap();
        let daily_retention = self
            .daily_retention
            .map(Duration::from_std)
            .map(Result::unwrap);
        let monthly_retention = self
            .monthly_retention
            .map(Duration::from_std)
            .map(Result::unwrap);
        let yearly_retention = self
            .yearly_retention
            .map(Duration::from_std)
            .map(Result::unwrap);
        let mut last_keep = None;

        let mut all_items: Vec<_> = iter.into_iter().collect::<Vec<_>>();

        // Decrease time sorting
        all_items.sort_by(|a, b| b.as_ref().date_time.cmp(&a.as_ref().date_time));

        tracing::info!(
            "Evaluating retention policy for {} backups at {}",
            all_items.len(),
            now
        );

        let max_deletions = all_items.len().saturating_sub(self.min_backups);
        tracing::info!(
            "Maximum deletions allowed: {} (keeping minimum {} backups)",
            max_deletions,
            self.min_backups
        );

        if max_deletions == 0 {
            tracing::info!("No backups to delete - at or below minimum backup count");
            return Vec::new();
        }

        let deletion_candidates: Vec<_> = all_items
            .into_iter()
            .filter(move |r| {
                let utc_date_time = r.as_ref().date_time.to_utc();
                tracing::debug!("Checking backup age: {:?}", utc_date_time);
                let age = now.signed_duration_since(utc_date_time);
                if age < default_retention {
                    tracing::debug!("Backup within default retention, keeping");
                    return false;
                }

                let should_keep = should_keep(
                    &utc_date_time,
                    age,
                    &mut last_keep,
                    yearly_retention,
                    DateTime::year,
                ) || should_keep(
                    &utc_date_time,
                    age,
                    &mut last_keep,
                    monthly_retention,
                    DateTime::month,
                ) || should_keep(
                    &utc_date_time,
                    age,
                    &mut last_keep,
                    daily_retention,
                    DateTime::day,
                );

                tracing::debug!("Backup retention decision made");
                !should_keep
            })
            .collect();

        let final_deletions: Vec<_> = deletion_candidates
            .into_iter()
            .rev()
            .take(max_deletions)
            .collect();
        tracing::info!(
            "Retention policy determined {} backups for deletion",
            final_deletions.len()
        );
        final_deletions
    }
}

fn should_keep<O: Copy, T: TimeZone<Offset = O>, R: Ord, F: Fn(&DateTime<T>) -> R>(
    to_check: &DateTime<T>,
    age: Duration,
    last_keep: &mut Option<DateTime<T>>,
    retention: Option<Duration>,
    cmp_value_extract_fn: F,
) -> bool {
    tracing::trace!("Retention check - last keep: {:?}", last_keep);
    match retention {
        None => false,
        Some(retention) => {
            if age < retention {
                match last_keep {
                    None => {
                        *last_keep = Some(*to_check);
                        true
                    }
                    Some(last_keep_val) => {
                        if cmp_value_extract_fn(to_check) < cmp_value_extract_fn(last_keep_val) {
                            *last_keep = Some(*to_check);
                            true
                        } else {
                            false
                        }
                    }
                }
            } else {
                false
            }
        }
    }
}

/// Associates an item with a timestamp for retention management
///
/// Used to track backup files with their creation times for retention policy
/// evaluation. Generic over both the item type and timezone.
#[derive(Clone, Debug, Hash, Eq, PartialEq, Builder, Getters)]
#[getset(get = "pub")]
pub struct ItemWithDateTime<R, T: TimeZone = Utc> {
    item: R,
    date_time: DateTime<T>,
}



impl<T: TimeZone> From<DateTime<T>> for ItemWithDateTime<(), T> {
    fn from(value: DateTime<T>) -> Self {
        Self::from(((), value))
    }
}

impl<R, T: TimeZone, D: Into<DateTime<T>>> From<(R, D)> for ItemWithDateTime<R, T> {
    fn from(value: (R, D)) -> Self {
        Self::builder()
            .item(value.0)
            .date_time(value.1.into())
            .build()
    }
}

impl<R, T: TimeZone> AsRef<ItemWithDateTime<R, T>> for ItemWithDateTime<R, T> {
    fn as_ref(&self) -> &ItemWithDateTime<R, T> {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::time::Duration as StdDuration;

    fn create_test_retention_config() -> RetentionConfig {
        RetentionConfig::builder()
            .default_retention(StdDuration::from_secs(7 * 24 * 3600)) // 7 days
            .daily_retention(StdDuration::from_secs(30 * 24 * 3600)) // 30 days
            .monthly_retention(StdDuration::from_secs(365 * 24 * 3600)) // 1 year
            .yearly_retention(StdDuration::from_secs(5 * 365 * 24 * 3600)) // 5 years
            .min_backups(3)
            .build()
    }

    #[test]
    fn test_retention_config_default() {
        let config = RetentionConfig::default();
        assert_eq!(config.min_backups, 3);
        assert_eq!(config.default_retention, StdDuration::from_secs(0));
        assert!(config.daily_retention.is_none());
        assert!(config.monthly_retention.is_none());
        assert!(config.yearly_retention.is_none());
    }

    #[test]
    fn test_min_backups_safety_net() {
        let config = RetentionConfig::builder()
            .default_retention(StdDuration::from_secs(1)) // 1 second (very short)
            .min_backups(3)
            .build();

        let now = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let old_backups: Vec<_> = (0..5)
            .map(|i| {
                let dt = now - Duration::days(i + 10); // All very old
                ItemWithDateTime::builder()
                    .item(format!("backup_{}", i))
                    .date_time(dt)
                    .build()
            })
            .collect();

        let to_delete = config.get_delete(old_backups, now);

        // Should only delete 2 backups (5 total - 3 min_backups)
        assert_eq!(to_delete.len(), 2);
    }

    #[test]
    fn test_default_retention_only() {
        let config = RetentionConfig::builder()
            .default_retention(StdDuration::from_secs(7 * 24 * 3600)) // 7 days
            .min_backups(1)
            .build();

        let now = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let backups = [
            ItemWithDateTime::builder()
                .item("recent")
                .date_time(now - Duration::days(3))
                .build(),
            ItemWithDateTime::builder()
                .item("old")
                .date_time(now - Duration::days(10))
                .build(),
        ];

        let to_delete = config.get_delete(backups.iter(), now);

        // Only the old backup should be deleted
        assert_eq!(to_delete.len(), 1);
        assert_eq!(to_delete[0].item, "old");
    }

    #[test]
    fn test_daily_retention() {
        let config = RetentionConfig::builder()
            .default_retention(StdDuration::from_secs(24 * 3600)) // 1 day
            .daily_retention(StdDuration::from_secs(7 * 24 * 3600)) // 7 days
            .min_backups(1)
            .build();

        let now = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let backups = [
            ItemWithDateTime::builder()
                .item("day5_backup1")
                .date_time(now - Duration::days(5) - Duration::hours(2))
                .build(),
            ItemWithDateTime::builder()
                .item("day5_backup2")
                .date_time(now - Duration::days(5) - Duration::hours(1))
                .build(),
            // One backup from 10 days ago (outside daily retention)
            ItemWithDateTime::builder()
                .item("day10_backup")
                .date_time(now - Duration::days(10))
                .build(),
        ];

        let to_delete = config.get_delete(backups.iter(), now);

        // Should delete the older backup from day 5 and the backup from day 10
        assert_eq!(to_delete.len(), 2);
    }

    #[test]
    fn test_monthly_retention() {
        let config = RetentionConfig::builder()
            .default_retention(StdDuration::from_secs(7 * 24 * 3600)) // 7 days
            .monthly_retention(StdDuration::from_secs(90 * 24 * 3600)) // 90 days
            .min_backups(1)
            .build();

        let now = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let backups = [
            ItemWithDateTime::builder()
                .item("month1_backup1")
                .date_time(now - Duration::days(30))
                .build(),
            ItemWithDateTime::builder()
                .item("month1_backup2")
                .date_time(now - Duration::days(35))
                .build(),
            // One backup from 120 days ago (outside monthly retention)
            ItemWithDateTime::builder()
                .item("old_backup")
                .date_time(now - Duration::days(120))
                .build(),
        ];

        let to_delete = config.get_delete(backups.iter(), now);

        // Should keep the most recent backup from the month and delete others
        assert!(!to_delete.is_empty());
    }

    #[test]
    fn test_yearly_retention() {
        let config = RetentionConfig::builder()
            .default_retention(StdDuration::from_secs(30 * 24 * 3600)) // 30 days
            .yearly_retention(StdDuration::from_secs(3 * 365 * 24 * 3600)) // 3 years
            .min_backups(1)
            .build();

        let now = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let backups = [
            ItemWithDateTime::builder()
                .item("year1_backup1")
                .date_time(now - Duration::days(365))
                .build(),
            ItemWithDateTime::builder()
                .item("year1_backup2")
                .date_time(now - Duration::days(370))
                .build(),
            // One backup from 4 years ago (outside yearly retention)
            ItemWithDateTime::builder()
                .item("old_backup")
                .date_time(now - Duration::days(4 * 365))
                .build(),
        ];

        let to_delete = config.get_delete(backups.iter(), now);

        // Should keep the most recent backup from the year and delete others
        assert!(!to_delete.is_empty());
    }

    #[test]
    fn test_complex_retention_scenario() {
        let config = create_test_retention_config();
        let now = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();

        let backups = [
            ItemWithDateTime::builder()
                .item("recent1")
                .date_time(now - Duration::days(1))
                .build(),
            ItemWithDateTime::builder()
                .item("recent2")
                .date_time(now - Duration::days(2))
                .build(),
            // Daily retention candidates
            ItemWithDateTime::builder()
                .item("daily1")
                .date_time(now - Duration::days(15))
                .build(),
            ItemWithDateTime::builder()
                .item("daily2")
                .date_time(now - Duration::days(16))
                .build(), // Same day, should be deleted
            // Monthly retention candidates
            ItemWithDateTime::builder()
                .item("monthly1")
                .date_time(now - Duration::days(60))
                .build(),
            ItemWithDateTime::builder()
                .item("monthly2")
                .date_time(now - Duration::days(65))
                .build(), // Same month, should be deleted
            // Very old backup (outside all retention)
            ItemWithDateTime::builder()
                .item("very_old")
                .date_time(now - Duration::days(2000))
                .build(),
        ];

        let to_delete = config.get_delete(backups.iter(), now);

        // Should delete some backups but keep recent ones and representative samples
        assert!(!to_delete.is_empty());
        assert!(to_delete.len() < backups.len());
    }

    #[test]
    fn test_empty_backup_list() {
        let config = create_test_retention_config();
        let now = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let backups: Vec<ItemWithDateTime<&str, Utc>> = vec![];

        let to_delete = config.get_delete(backups.iter(), now);

        assert_eq!(to_delete.len(), 0);
    }
}
