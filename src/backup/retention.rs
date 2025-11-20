use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::cmp::Reverse;
use std::fmt::{Debug, Formatter};
use std::rc::Rc;
use validator::Validate;

/// Configuration for backup retention policies
/// 
/// Defines how long different types of backups should be kept:
/// - default_retention: Applied to all backups
/// - daily_retention: Special retention for daily backups (keeps one per day)
/// - monthly_retention: Special retention for monthly backups (keeps one per month)  
/// - yearly_retention: Special retention for yearly backups (keeps one per year)
/// 
/// The algorithm keeps the most recent backup in each time category,
/// allowing for grandfather-father-son backup rotation schemes.
#[skip_serializing_none]
#[derive(Clone, Default, Validate, Serialize, Deserialize, Debug)]
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
}

impl RetentionConfig {
    /// Determines which backups should be deleted based on retention policy
    /// 
    /// This is the core retention algorithm that implements grandfather-father-son
    /// backup rotation. It:
    /// 
    /// 1. Applies default retention to all backups
    /// 2. Preserves the most recent backup from each day/month/year
    /// 3. Returns an iterator of backups that should be deleted
    /// 
    /// The algorithm ensures that even if a backup is older than default_retention,
    /// it will be kept if it's the most recent backup for its time period
    /// (daily/monthly/yearly) and within that retention window.
    pub fn get_delete<R, T, I, II>(
        &self,
        iter: I,
        now: DateTime<Utc>,
    ) -> Box<dyn Iterator<Item = II>>
    where
        R: 'static,
        T: TimeZone + 'static,
        II: AsRef<ItemWithDateTime<R, T>> + 'static,
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

        let iter = iter
            .into_iter()
            .sorted_unstable_by_key(|r| Reverse(r.as_ref().date_time.clone()))
            .filter(move |r| {
                let utc_date_time = r.as_ref().date_time.to_utc();
                println!("{:?}", utc_date_time);
                let age = now.signed_duration_since(utc_date_time);
                if age < default_retention {
                    println!();
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

                println!();
                return !should_keep;
            });

        Box::new(iter)
    }
}

fn should_keep<O: Copy, T: TimeZone<Offset = O>, R: Ord, F: Fn(&DateTime<T>) -> R>(
    to_check: &DateTime<T>,
    age: Duration,
    last_keep: &mut Option<DateTime<T>>,
    retention: Option<Duration>,
    cmp_value_extract_fn: F,
) -> bool {
    println!("last keep {:?}", last_keep);
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

#[derive(Clone, Hash, Eq, PartialEq)]
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

impl<T: TimeZone> Debug for ItemWithDateTime<(), T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.date_time.fmt(f)
    }
}
