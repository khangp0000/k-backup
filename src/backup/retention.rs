use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::cmp::Reverse;
use std::fmt::Debug;
use std::rc::Rc;
use validator::Validate;

fn default_min_backups() -> usize {
    3
}

/// Configuration for backup retention policies
/// 
/// Defines how long different types of backups should be kept:
/// - default_retention: Applied to all backups
/// - daily_retention: Special retention for daily backups (keeps one per day)
/// - monthly_retention: Special retention for monthly backups (keeps one per month)  
/// - yearly_retention: Special retention for yearly backups (keeps one per year)
/// - min_backups: Minimum number of backups to always keep (safety net)
/// 
/// The algorithm keeps the most recent backup in each time category,
/// allowing for grandfather-father-son backup rotation schemes.
#[skip_serializing_none]
#[derive(Clone, Validate, Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct RetentionConfig {
    /// Base retention period applied to all backups
    /// 
    /// Backups older than this duration are eligible for deletion,
    /// unless they're preserved by daily/monthly/yearly retention rules.
    #[serde(with = "humantime_serde")]
    pub default_retention: std::time::Duration,
    
    /// How long to keep daily backups (one per day)
    /// 
    /// The most recent backup from each day within this period is preserved.
    /// Example: "7days" keeps one backup per day for the last week.
    #[serde(with = "humantime_serde")]
    pub daily_retention: Option<std::time::Duration>,
    
    /// How long to keep monthly backups (one per month)
    /// 
    /// The most recent backup from each month within this period is preserved.
    /// Example: "3months" keeps one backup per month for the last 3 months.
    #[serde(with = "humantime_serde")]
    pub monthly_retention: Option<std::time::Duration>,
    
    /// How long to keep yearly backups (one per year)
    /// 
    /// The most recent backup from each year within this period is preserved.
    /// Example: "5years" keeps one backup per year for the last 5 years.
    #[serde(with = "humantime_serde")]
    pub yearly_retention: Option<std::time::Duration>,
    
    /// Minimum number of backups to always keep
    /// 
    /// Safety net to prevent all backups from being deleted if the system
    /// hasn't run for a long time. Always keeps at least this many of the
    /// most recent backups, regardless of age.
    #[serde(default = "default_min_backups")]
    pub min_backups: usize,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            default_retention: std::time::Duration::from_secs(0),
            daily_retention: None,
            monthly_retention: None,
            yearly_retention: None,
            min_backups: default_min_backups(),
        }
    }
}

impl RetentionConfig {
    /// Determines which backups should be deleted based on retention policy
    /// 
    /// This is the core retention algorithm that implements grandfather-father-son
    /// backup rotation. It:
    /// 
    /// 1. Applies default retention to all backups
    /// 2. Preserves the most recent backup from each day/month/year
    /// 3. Ensures at least min_backups are always kept (safety net)
    /// 4. Returns an iterator of backups that should be deleted
    /// 
    /// The algorithm ensures that even if a backup is older than default_retention,
    /// it will be kept if it's the most recent backup for its time period
    /// (daily/monthly/yearly) and within that retention window.
    pub fn get_delete<R, T, I, II>(
        &self,
        iter: I,
        now: DateTime<Utc>,
    ) -> Vec<II>
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

        let all_items: Vec<_> = iter
            .into_iter()
            .sorted_unstable_by_key(|r| Reverse(r.as_ref().date_time.clone()))
            .collect();
        
        tracing::info!("Evaluating retention policy for {} backups at {}", all_items.len(), now);
        
        let max_deletions = all_items.len().saturating_sub(self.min_backups);
        tracing::info!("Maximum deletions allowed: {} (keeping minimum {} backups)", max_deletions, self.min_backups);
        
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
                return !should_keep;
            })
            .collect();

        let final_deletions: Vec<_> = deletion_candidates.into_iter().rev().take(max_deletions).collect();
        tracing::info!("Retention policy determined {} backups for deletion", final_deletions.len());
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
                        if cmp_value_extract_fn(&to_check) < cmp_value_extract_fn(last_keep_val) {
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

#[derive(Clone, Hash, Eq, PartialEq, Debug)]
pub struct ItemWithDateTime<R, T: TimeZone> {
    pub item: R,
    pub date_time: Rc<DateTime<T>>,
}

impl<T: TimeZone> ItemWithDateTime<(), T> {
    fn new(date_time: DateTime<T>) -> Self {
        ItemWithDateTime {
            item: (),
            date_time: date_time.into(),
        }
    }
}

impl<T: TimeZone> From<DateTime<T>> for ItemWithDateTime<(), T> {
    fn from(value: DateTime<T>) -> Self {
        Self::new(value)
    }
}

impl<R, T: TimeZone> From<(R, DateTime<T>)> for ItemWithDateTime<R, T> {
    fn from(value: (R, DateTime<T>)) -> Self {
        Self {
            item: value.0,
            date_time: Rc::new(value.1),
        }
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
        RetentionConfig {
            default_retention: StdDuration::from_secs(7 * 24 * 3600), // 7 days
            daily_retention: Some(StdDuration::from_secs(30 * 24 * 3600)), // 30 days
            monthly_retention: Some(StdDuration::from_secs(365 * 24 * 3600)), // 1 year
            yearly_retention: Some(StdDuration::from_secs(5 * 365 * 24 * 3600)), // 5 years
            min_backups: 3,
        }
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
        let config = RetentionConfig {
            default_retention: StdDuration::from_secs(1), // 1 second (very short)
            daily_retention: None,
            monthly_retention: None,
            yearly_retention: None,
            min_backups: 3,
        };

        let now = Utc::now();
        let old_backups: Vec<_> = (0..5)
            .map(|i| {
                let dt = now - Duration::days(i + 10); // All very old
                ItemWithDateTime::from((format!("backup_{}", i), dt))
            })
            .collect();

        let to_delete = config.get_delete(old_backups.iter(), now);
        
        // Should only delete 2 backups (5 total - 3 min_backups)
        assert_eq!(to_delete.len(), 2);
    }

    #[test]
    fn test_default_retention_only() {
        let config = RetentionConfig {
            default_retention: StdDuration::from_secs(7 * 24 * 3600), // 7 days
            daily_retention: None,
            monthly_retention: None,
            yearly_retention: None,
            min_backups: 1,
        };

        let now = Utc::now();
        let backups = vec![
            ItemWithDateTime::from(("recent", now - Duration::days(3))),
            ItemWithDateTime::from(("old", now - Duration::days(10))),
        ];

        let to_delete = config.get_delete(backups.iter(), now);
        
        // Only the old backup should be deleted
        assert_eq!(to_delete.len(), 1);
        assert_eq!(to_delete[0].item, "old");
    }

    #[test]
    fn test_daily_retention() {
        let config = RetentionConfig {
            default_retention: StdDuration::from_secs(1 * 24 * 3600), // 1 day
            daily_retention: Some(StdDuration::from_secs(7 * 24 * 3600)), // 7 days
            monthly_retention: None,
            yearly_retention: None,
            min_backups: 1,
        };

        let now = Utc::now();
        let backups = vec![
            // Two backups from the same day (5 days ago)
            ItemWithDateTime::from(("day5_backup1", now - Duration::days(5) - Duration::hours(2))),
            ItemWithDateTime::from(("day5_backup2", now - Duration::days(5) - Duration::hours(1))),
            // One backup from 10 days ago (outside daily retention)
            ItemWithDateTime::from(("day10_backup", now - Duration::days(10))),
        ];

        let to_delete = config.get_delete(backups.iter(), now);
        
        // Should delete the older backup from day 5 and the backup from day 10
        assert_eq!(to_delete.len(), 2);
    }

    #[test]
    fn test_monthly_retention() {
        let config = RetentionConfig {
            default_retention: StdDuration::from_secs(7 * 24 * 3600), // 7 days
            daily_retention: None,
            monthly_retention: Some(StdDuration::from_secs(90 * 24 * 3600)), // 90 days
            yearly_retention: None,
            min_backups: 1,
        };

        let now = Utc::now();
        let backups = vec![
            // Two backups from the same month (30 days ago)
            ItemWithDateTime::from(("month1_backup1", now - Duration::days(30))),
            ItemWithDateTime::from(("month1_backup2", now - Duration::days(35))),
            // One backup from 120 days ago (outside monthly retention)
            ItemWithDateTime::from(("old_backup", now - Duration::days(120))),
        ];

        let to_delete = config.get_delete(backups.iter(), now);
        
        // Should keep the most recent backup from the month and delete others
        assert!(to_delete.len() >= 1);
    }

    #[test]
    fn test_yearly_retention() {
        let config = RetentionConfig {
            default_retention: StdDuration::from_secs(30 * 24 * 3600), // 30 days
            daily_retention: None,
            monthly_retention: None,
            yearly_retention: Some(StdDuration::from_secs(3 * 365 * 24 * 3600)), // 3 years
            min_backups: 1,
        };

        let now = Utc::now();
        let backups = vec![
            // Two backups from the same year (1 year ago)
            ItemWithDateTime::from(("year1_backup1", now - Duration::days(365))),
            ItemWithDateTime::from(("year1_backup2", now - Duration::days(370))),
            // One backup from 4 years ago (outside yearly retention)
            ItemWithDateTime::from(("old_backup", now - Duration::days(4 * 365))),
        ];

        let to_delete = config.get_delete(backups.iter(), now);
        
        // Should keep the most recent backup from the year and delete others
        assert!(to_delete.len() >= 1);
    }

    #[test]
    fn test_complex_retention_scenario() {
        let config = create_test_retention_config();
        let now = Utc::now();
        
        let backups = vec![
            // Recent backups (within default retention)
            ItemWithDateTime::from(("recent1", now - Duration::days(1))),
            ItemWithDateTime::from(("recent2", now - Duration::days(2))),
            
            // Daily retention candidates
            ItemWithDateTime::from(("daily1", now - Duration::days(15))),
            ItemWithDateTime::from(("daily2", now - Duration::days(16))), // Same day, should be deleted
            
            // Monthly retention candidates
            ItemWithDateTime::from(("monthly1", now - Duration::days(60))),
            ItemWithDateTime::from(("monthly2", now - Duration::days(65))), // Same month, should be deleted
            
            // Very old backup (outside all retention)
            ItemWithDateTime::from(("very_old", now - Duration::days(2000))),
        ];

        let to_delete = config.get_delete(backups.iter(), now);
        
        // Should delete some backups but keep recent ones and representative samples
        assert!(to_delete.len() > 0);
        assert!(to_delete.len() < backups.len());
    }

    #[test]
    fn test_empty_backup_list() {
        let config = create_test_retention_config();
        let now = Utc::now();
        let backups: Vec<ItemWithDateTime<&str, Utc>> = vec![];

        let to_delete = config.get_delete(backups.iter(), now);
        
        assert_eq!(to_delete.len(), 0);
    }

    #[test]
    fn test_item_with_datetime_creation() {
        let now = Utc::now();
        
        // Test From<DateTime<T>>
        let item1: ItemWithDateTime<(), Utc> = now.into();
        assert_eq!(*item1.date_time, now);
        
        // Test From<(R, DateTime<T>)>
        let item2: ItemWithDateTime<String, Utc> = ("test".to_string(), now).into();
        assert_eq!(item2.item, "test");
        assert_eq!(*item2.date_time, now);
    }

    #[test]
    fn test_item_with_datetime_equality() {
        let now = Utc::now();
        let item1 = ItemWithDateTime::from(("test", now));
        let item2 = ItemWithDateTime::from(("test", now));
        
        assert_eq!(item1, item2);
    }
}