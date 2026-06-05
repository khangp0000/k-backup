//! Grandfather-father-son retention logic.

use crate::config::RetentionConfig;
use chrono::{DateTime, Datelike, Duration, Utc};
use std::path::Path;
use std::rc::Rc;

/// A backup file with its parsed timestamp.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BackupFile {
    pub path: Rc<Path>,
    pub timestamp: DateTime<Utc>,
}

/// Returns list of backup files that should be deleted according to retention policy.
pub fn get_deletions(
    backups: &[&BackupFile],
    now: DateTime<Utc>,
    config: &RetentionConfig,
) -> Vec<Rc<Path>> {
    if backups.len() <= config.min_backups {
        return Vec::new();
    }

    let default_retention = Duration::from_std(config.default_retention).unwrap();
    let daily = config
        .daily_retention
        .map(|d| Duration::from_std(d).unwrap());
    let weekly = config
        .weekly_retention
        .map(|d| Duration::from_std(d).unwrap());
    let monthly = config
        .monthly_retention
        .map(|d| Duration::from_std(d).unwrap());
    let yearly = config
        .yearly_retention
        .map(|d| Duration::from_std(d).unwrap());

    // Sort newest first
    let mut sorted: Vec<&BackupFile> = backups.to_vec();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.timestamp));

    let mut last_daily: Option<(i32, u32, u32)> = None;
    let mut last_weekly: Option<(i32, u32)> = None;
    let mut last_monthly: Option<(i32, u32)> = None;
    let mut last_yearly: Option<i32> = None;

    let mut to_delete: Vec<Rc<Path>> = Vec::new();

    for backup in &sorted {
        let age = now - backup.timestamp;
        let dt = backup.timestamp;

        // Within default retention — always keep, but claim GFS slots
        if age <= default_retention {
            last_daily = Some((dt.year(), dt.month(), dt.day()));
            last_weekly = Some((dt.iso_week().year(), dt.iso_week().week()));
            last_monthly = Some((dt.year(), dt.month()));
            last_yearly = Some(dt.year());
            continue;
        }

        let mut keep = false;

        // Yearly
        if let Some(yr) = yearly {
            if age <= yr {
                let key = dt.year();
                if last_yearly != Some(key) {
                    last_yearly = Some(key);
                    keep = true;
                }
            }
        }

        // Monthly
        if !keep {
            if let Some(mr) = monthly {
                if age <= mr {
                    let key = (dt.year(), dt.month());
                    if last_monthly != Some(key) {
                        last_monthly = Some(key);
                        keep = true;
                    }
                }
            }
        }

        // Weekly
        if !keep {
            if let Some(wr) = weekly {
                if age <= wr {
                    let key = (dt.iso_week().year(), dt.iso_week().week());
                    if last_weekly != Some(key) {
                        last_weekly = Some(key);
                        keep = true;
                    }
                }
            }
        }

        // Daily
        if !keep {
            if let Some(dr) = daily {
                if age <= dr {
                    let key = (dt.year(), dt.month(), dt.day());
                    if last_daily != Some(key) {
                        last_daily = Some(key);
                        keep = true;
                    }
                }
            }
        }

        if !keep {
            to_delete.push(Rc::clone(&backup.path));
        }
    }

    // Safety net: never delete below min_backups
    let total_remaining = sorted.len() - to_delete.len();
    if total_remaining < config.min_backups {
        let excess = config.min_backups - total_remaining;
        to_delete.truncate(to_delete.len().saturating_sub(excess));
    }

    to_delete
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::time::Duration as StdDuration;

    fn make_config(default_days: u64, min_backups: usize) -> RetentionConfig {
        RetentionConfig {
            default_retention: StdDuration::from_secs(default_days * 86400),
            daily_retention: None,
            weekly_retention: None,
            monthly_retention: None,
            yearly_retention: None,
            min_backups,
        }
    }

    fn make_backup(path: &str, days_ago: i64, now: DateTime<Utc>) -> BackupFile {
        BackupFile {
            path: Rc::from(Path::new(path)),
            timestamp: now - Duration::days(days_ago),
        }
    }

    #[test]
    fn default_retention_removes_old() {
        let now = Utc::now();
        let b1 = make_backup("new.age", 1, now);
        let b2 = make_backup("old.age", 30, now);

        let config = make_config(7, 1);
        let backups: Vec<&BackupFile> = vec![&b1, &b2];
        let deletions = get_deletions(&backups, now, &config);
        assert_eq!(deletions, vec![Rc::from(Path::new("old.age"))]);
    }

    #[test]
    fn respects_min_backups() {
        let now = Utc::now();
        let b1 = make_backup("a.age", 30, now);
        let b2 = make_backup("b.age", 31, now);

        let config = make_config(7, 3); // min_backups=3 > total=2
        let backups: Vec<&BackupFile> = vec![&b1, &b2];
        let deletions = get_deletions(&backups, now, &config);
        assert!(deletions.is_empty());
    }

    #[test]
    fn daily_keeps_one_per_day() {
        let now = Utc::now();
        // Two backups on the same day (8 days ago), one on different day (9 days ago)
        let b1 = make_backup("recent.age", 1, now);
        let b2 = make_backup("day8_a.age", 8, now);
        let b3 = make_backup("day9.age", 9, now);

        let mut config = make_config(7, 1);
        config.daily_retention = Some(StdDuration::from_secs(30 * 86400));

        let backups: Vec<&BackupFile> = vec![&b1, &b2, &b3];
        let deletions = get_deletions(&backups, now, &config);
        // All should be kept: b1 within default, b2 daily for day8, b3 daily for day9
        assert!(deletions.is_empty());
    }

    #[test]
    fn empty_list_returns_empty() {
        let now = Utc::now();
        let config = make_config(7, 3);
        let backups: Vec<&BackupFile> = vec![];
        let deletions = get_deletions(&backups, now, &config);
        assert!(deletions.is_empty());
    }

    #[test]
    fn test_default_retention_claims_gfs_slots() {
        // A backup within default_retention for June should prevent keeping
        // an older June backup via monthly_retention (same month already claimed)
        let now = Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap();

        let config = RetentionConfig {
            default_retention: std::time::Duration::from_secs(7 * 86400), // 7 days
            daily_retention: None,
            weekly_retention: None,
            monthly_retention: Some(std::time::Duration::from_secs(90 * 86400)), // 90 days
            yearly_retention: None,
            min_backups: 1,
        };

        let recent = BackupFile {
            path: Rc::from(Path::new("/backups/recent.tar")),
            timestamp: now - chrono::Duration::days(2), // June 13
        };
        let older = BackupFile {
            path: Rc::from(Path::new("/backups/older_june.tar")),
            timestamp: now - chrono::Duration::days(10), // June 5
        };
        let backups: Vec<&BackupFile> = vec![&recent, &older];

        let deletions = get_deletions(&backups, now, &config);
        // The older June backup should be deleted because the recent one
        // already claims the June monthly slot
        assert!(
            deletions.contains(&Rc::from(Path::new("/backups/older_june.tar"))),
            "Older same-month backup should be deleted (slot claimed by default retention backup)"
        );
    }
}
