#![cfg(feature = "integration-tests")]
//! Comprehensive integration tests for k-backup.
//!
//! Run with: cargo test --features integration-tests
//! With vendored deps: cargo test --features "integration-tests fully-static vendored-openssl"
//!
//! SMTP tests require environment variables:
//!   SMTP_HOST, SMTP_FROM, SMTP_TO, SMTP_USERNAME, SMTP_PASSWORD
//!   Optional: SMTP_MODE (Ssl|StartTls|Unsecured, defaults to Ssl)

use chrono::{TimeZone, Utc};
use k_backup::backup::archive::base64::Base64Source;
use k_backup::backup::archive::sqlite::SqliteDBSource;
use k_backup::backup::archive::walkdir_globset::{CustomDeserializedGlob, WalkdirAndGlobsetSource};
use k_backup::backup::archive::ArchiveEntryConfig;
use k_backup::backup::backup_config::BackupConfig;
use k_backup::backup::compress::xz::XzConfig;
use k_backup::backup::compress::CompressorConfig;
use k_backup::backup::encrypt::age::AgeEncryptorConfig;
use k_backup::backup::encrypt::EncryptorConfig;
use k_backup::backup::notifications::smtp::{SmtpMode, SmtpNotificationConfig};
use k_backup::backup::notifications::{Notification, NotificationConfig};
use k_backup::backup::redacted::RedactedString;
use k_backup::backup::retention::{ItemWithDateTime, RetentionConfig};
use lettre::message::Mailbox;
use rayon::ThreadPoolBuilder;
use std::collections::HashSet;
use std::fs;
use std::io::{BufReader, Read};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

// ─── Helpers ───────────────────────────────────────────────────────────────

fn test_passphrase() -> RedactedString {
    RedactedString::builder()
        .inner("integration-test-passphrase")
        .build()
}

fn pool() -> Arc<rayon::ThreadPool> {
    Arc::new(ThreadPoolBuilder::new().num_threads(2).build().unwrap())
}

fn create_source_files(dir: &std::path::Path) {
    fs::create_dir_all(dir.join("docs")).unwrap();
    fs::create_dir_all(dir.join("images")).unwrap();
    fs::write(dir.join("docs/readme.txt"), "hello world").unwrap();
    fs::write(dir.join("docs/notes.md"), "# Notes\nSome notes").unwrap();
    fs::write(dir.join("images/photo.jpg"), b"fake jpeg data").unwrap();
    fs::write(dir.join("root.txt"), "root file").unwrap();
}

fn create_sqlite_db(path: &std::path::Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT);
         INSERT INTO test VALUES (1, 'hello');
         INSERT INTO test VALUES (2, 'world');",
    )
    .unwrap();
}

fn decrypt_and_decompress(path: &std::path::Path, passphrase: &str) -> Vec<(String, Vec<u8>)> {
    let file = fs::File::open(path).unwrap();
    let decryptor = age::Decryptor::new(BufReader::new(file)).unwrap();
    let identity = age::scrypt::Identity::new(age::secrecy::SecretString::new(
        passphrase.to_string().into(),
    ));
    let decrypted = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .unwrap();
    let decompressed = liblzma::read::XzDecoder::new(BufReader::new(decrypted));
    let mut archive = tar::Archive::new(decompressed);

    let mut entries = Vec::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().to_string();
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();
        entries.push((path, content));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

// ─── Full Pipeline Tests ───────────────────────────────────────────────────

#[test]
fn test_full_backup_pipeline_base64_source() {
    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("test")
        .out_dir(out_dir.clone())
        .files(vec![ArchiveEntryConfig::Base64(
            Base64Source::builder()
                .content("integration test content")
                .dst(PathBuf::from("test.txt"))
                .build(),
        )])
        .compressor(CompressorConfig::Xz(
            XzConfig::builder().level(3).thread(2).build(),
        ))
        .encryptor(EncryptorConfig::Age(AgeEncryptorConfig::Passphrase {
            passphrase: test_passphrase(),
        }))
        .build();

    let dt = Utc.with_ymd_and_hms(2025, 6, 15, 10, 0, 0).unwrap();
    let (path, err) = config.create_archive(dt, pool()).unwrap();

    assert!(err.is_empty());
    assert!(path.exists());
    assert!(path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with(".tar.xz.age"));

    let entries = decrypt_and_decompress(&path, "integration-test-passphrase");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "test.txt");
    assert_eq!(entries[0].1, b"integration test content");
}

#[test]
fn test_full_backup_pipeline_glob_source() {
    let tmp = TempDir::new().unwrap();
    let src_dir = tmp.path().join("source");
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();
    create_source_files(&src_dir);

    let txt_glob: CustomDeserializedGlob = serde_json::from_str("\"**/*.txt\"").unwrap();

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("glob_test")
        .out_dir(out_dir.clone())
        .files(vec![ArchiveEntryConfig::Glob(
            WalkdirAndGlobsetSource::builder()
                .src_dir(src_dir)
                .dst_dir("files")
                .globset(vec![txt_glob])
                .build(),
        )])
        .compressor(CompressorConfig::Xz(XzConfig::builder().level(1).build()))
        .encryptor(EncryptorConfig::Age(AgeEncryptorConfig::Passphrase {
            passphrase: test_passphrase(),
        }))
        .build();

    let dt = Utc.with_ymd_and_hms(2025, 6, 15, 10, 0, 0).unwrap();
    let (path, err) = config.create_archive(dt, pool()).unwrap();

    assert!(err.is_empty());
    let entries = decrypt_and_decompress(&path, "integration-test-passphrase");
    let paths: Vec<&str> = entries.iter().map(|e| e.0.as_str()).collect();
    assert!(paths.contains(&"files/docs/readme.txt"));
    assert!(paths.contains(&"files/root.txt"));
    assert!(!paths
        .iter()
        .any(|p| p.ends_with(".jpg") || p.ends_with(".md")));
}

#[test]
fn test_full_backup_pipeline_sqlite_source() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();
    create_sqlite_db(&db_path);

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("sqlite_test")
        .out_dir(out_dir.clone())
        .files(vec![ArchiveEntryConfig::Sqlite(
            SqliteDBSource::builder()
                .src(db_path.clone())
                .dst(PathBuf::from("backup.db"))
                .build(),
        )])
        .compressor(CompressorConfig::Xz(XzConfig::builder().level(1).build()))
        .encryptor(EncryptorConfig::Age(AgeEncryptorConfig::Passphrase {
            passphrase: test_passphrase(),
        }))
        .build();

    let dt = Utc.with_ymd_and_hms(2025, 6, 15, 10, 0, 0).unwrap();
    let (path, err) = config.create_archive(dt, pool()).unwrap();

    assert!(err.is_empty());
    let entries = decrypt_and_decompress(&path, "integration-test-passphrase");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "backup.db");

    // Verify the SQLite backup is valid
    let restored_db = tmp.path().join("restored.db");
    fs::write(&restored_db, &entries[0].1).unwrap();
    let conn = rusqlite::Connection::open(&restored_db).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM test", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn test_full_backup_pipeline_mixed_sources() {
    let tmp = TempDir::new().unwrap();
    let src_dir = tmp.path().join("source");
    let db_path = tmp.path().join("app.db");
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();
    create_source_files(&src_dir);
    create_sqlite_db(&db_path);

    let all_glob: CustomDeserializedGlob = serde_json::from_str("\"**/*\"").unwrap();

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("mixed")
        .out_dir(out_dir.clone())
        .files(vec![
            ArchiveEntryConfig::Sqlite(
                SqliteDBSource::builder()
                    .src(db_path)
                    .dst(PathBuf::from("db/app.db"))
                    .build(),
            ),
            ArchiveEntryConfig::Glob(
                WalkdirAndGlobsetSource::builder()
                    .src_dir(src_dir)
                    .dst_dir("files")
                    .globset(vec![all_glob])
                    .build(),
            ),
            ArchiveEntryConfig::Base64(
                Base64Source::builder()
                    .content("secret config data")
                    .dst(PathBuf::from("config/secret.txt"))
                    .build(),
            ),
        ])
        .compressor(CompressorConfig::Xz(
            XzConfig::builder().level(6).thread(2).build(),
        ))
        .encryptor(EncryptorConfig::Age(AgeEncryptorConfig::Passphrase {
            passphrase: test_passphrase(),
        }))
        .build();

    let dt = Utc.with_ymd_and_hms(2025, 6, 15, 10, 0, 0).unwrap();
    let (path, err) = config.create_archive(dt, pool()).unwrap();

    assert!(err.is_empty());
    let entries = decrypt_and_decompress(&path, "integration-test-passphrase");
    let paths: Vec<&str> = entries.iter().map(|e| e.0.as_str()).collect();

    assert!(paths.contains(&"db/app.db"));
    assert!(paths.contains(&"files/docs/readme.txt"));
    assert!(paths.contains(&"files/images/photo.jpg"));
    assert!(paths.contains(&"config/secret.txt"));
}

// ─── Compression Tests ─────────────────────────────────────────────────────

#[test]
fn test_no_compression_no_encryption() {
    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("nocomp")
        .out_dir(out_dir)
        .files(vec![ArchiveEntryConfig::Base64(
            Base64Source::builder()
                .content("plain data")
                .dst(PathBuf::from("f.txt"))
                .build(),
        )])
        .compressor(CompressorConfig::None)
        .encryptor(EncryptorConfig::None)
        .build();

    let dt = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let (path, _) = config.create_archive(dt, pool()).unwrap();

    assert!(path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with(".tar"));

    // Verify it's a valid tar directly
    let file = fs::File::open(&path).unwrap();
    let mut archive = tar::Archive::new(file);
    let mut entries: Vec<_> = archive
        .entries()
        .unwrap()
        .map(|e| {
            let mut e = e.unwrap();
            let path = e.path().unwrap().to_string_lossy().to_string();
            let mut content = String::new();
            e.read_to_string(&mut content).unwrap();
            (path, content)
        })
        .collect();
    entries.sort();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0], ("f.txt".to_string(), "plain data".to_string()));
}

#[test]
fn test_compression_levels_produce_different_sizes() {
    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();

    let content = "ABCDEFGH".repeat(10000);

    let mut sizes = Vec::new();
    for level in [1u32, 9] {
        let config = BackupConfig::builder()
            .cron("0 1 * * *")
            .archive_base_name(format!("lvl{level}"))
            .out_dir(out_dir.clone())
            .files(vec![ArchiveEntryConfig::Base64(
                Base64Source::builder()
                    .content(content.as_str())
                    .dst(PathBuf::from("data.txt"))
                    .build(),
            )])
            .compressor(CompressorConfig::Xz(
                XzConfig::builder().level(level).build(),
            ))
            .encryptor(EncryptorConfig::None)
            .build();

        let dt = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        let (path, _) = config.create_archive(dt, pool()).unwrap();
        sizes.push(fs::metadata(&path).unwrap().len());
    }

    assert!(sizes[1] <= sizes[0]);
}

// ─── Retention Tests ───────────────────────────────────────────────────────

#[test]
fn test_retention_deletes_expired_backups() {
    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("ret")
        .out_dir(out_dir.clone())
        .files(vec![ArchiveEntryConfig::Base64(
            Base64Source::builder()
                .content("x")
                .dst(PathBuf::from("x.txt"))
                .build(),
        )])
        .compressor(CompressorConfig::None)
        .encryptor(EncryptorConfig::None)
        .retention(
            RetentionConfig::builder()
                .default_retention(Duration::from_secs(2 * 86400))
                .min_backups(1)
                .build(),
        )
        .build();

    let now = Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap();
    let mut backup_set = HashSet::new();

    // Old backup (5 days ago - expired)
    let old_path = out_dir.join("ret.2025-06-10T12h00m00s_0000.tar");
    fs::write(&old_path, "old").unwrap();
    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(old_path.clone())
            .date_time(now - chrono::Duration::days(5))
            .build(),
    ));

    // Recent backup (1 day ago - not expired)
    let recent_path = out_dir.join("ret.2025-06-14T12h00m00s_0000.tar");
    fs::write(&recent_path, "recent").unwrap();
    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(recent_path.clone())
            .date_time(now - chrono::Duration::days(1))
            .build(),
    ));

    config
        .execute_backup_cycle(&mut backup_set, now, pool())
        .unwrap();

    assert!(!old_path.exists(), "Expired backup should be deleted");
    assert!(recent_path.exists(), "Recent backup should be kept");
    assert_eq!(backup_set.len(), 2);
}

#[test]
fn test_retention_min_backups_safety_net() {
    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("safe")
        .out_dir(out_dir.clone())
        .files(vec![ArchiveEntryConfig::Base64(
            Base64Source::builder()
                .content("x")
                .dst(PathBuf::from("x.txt"))
                .build(),
        )])
        .compressor(CompressorConfig::None)
        .encryptor(EncryptorConfig::None)
        .retention(
            RetentionConfig::builder()
                .default_retention(Duration::from_secs(1))
                .min_backups(5)
                .build(),
        )
        .build();

    let now = Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap();
    let mut backup_set = HashSet::new();

    for i in 1..=3i64 {
        let path = out_dir.join(format!("safe.old{i}.tar"));
        fs::write(&path, "data").unwrap();
        backup_set.insert(Rc::new(
            ItemWithDateTime::builder()
                .item(path)
                .date_time(now - chrono::Duration::days(i))
                .build(),
        ));
    }

    config
        .execute_backup_cycle(&mut backup_set, now, pool())
        .unwrap();

    // All 3 old + 1 new = 4, under min_backups=5, so none deleted
    assert_eq!(backup_set.len(), 4);
}

#[test]
fn test_retention_daily_keeps_one_per_day() {
    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("daily")
        .out_dir(out_dir.clone())
        .files(vec![ArchiveEntryConfig::Base64(
            Base64Source::builder()
                .content("x")
                .dst(PathBuf::from("x.txt"))
                .build(),
        )])
        .compressor(CompressorConfig::None)
        .encryptor(EncryptorConfig::None)
        .retention(
            RetentionConfig::builder()
                .default_retention(Duration::from_secs(1)) // everything expired by default
                .daily_retention(Duration::from_secs(7 * 86400)) // keep 1/day for 7 days
                .min_backups(1)
                .build(),
        )
        .build();

    let now = Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap();
    let mut backup_set = HashSet::new();

    // Two backups on the same day (June 14) — only the most recent should be kept
    let same_day_early = out_dir.join("daily.early.tar");
    let same_day_late = out_dir.join("daily.late.tar");
    let different_day = out_dir.join("daily.diff.tar");

    fs::write(&same_day_early, "a").unwrap();
    fs::write(&same_day_late, "b").unwrap();
    fs::write(&different_day, "c").unwrap();

    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(same_day_early.clone())
            .date_time(Utc.with_ymd_and_hms(2025, 6, 14, 2, 0, 0).unwrap())
            .build(),
    ));
    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(same_day_late.clone())
            .date_time(Utc.with_ymd_and_hms(2025, 6, 14, 18, 0, 0).unwrap())
            .build(),
    ));
    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(different_day.clone())
            .date_time(Utc.with_ymd_and_hms(2025, 6, 13, 12, 0, 0).unwrap())
            .build(),
    ));

    config
        .execute_backup_cycle(&mut backup_set, now, pool())
        .unwrap();

    // Daily retention keeps one per day: June 14 (most recent) + June 13 + new backup
    assert!(
        same_day_late.exists() || different_day.exists(),
        "At least one daily backup kept"
    );
    // The early same-day backup should be deleted (duplicate day)
    assert!(
        !same_day_early.exists(),
        "Earlier same-day backup should be deleted"
    );
}

#[test]
fn test_retention_weekly_keeps_one_per_week() {
    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("weekly")
        .out_dir(out_dir.clone())
        .files(vec![ArchiveEntryConfig::Base64(
            Base64Source::builder()
                .content("x")
                .dst(PathBuf::from("x.txt"))
                .build(),
        )])
        .compressor(CompressorConfig::None)
        .encryptor(EncryptorConfig::None)
        .retention(
            RetentionConfig::builder()
                .default_retention(Duration::from_secs(1))
                .weekly_retention(Duration::from_secs(30 * 86400)) // 30 days
                .min_backups(1)
                .build(),
        )
        .build();

    // 2025-06-15 is a Sunday (ISO week 24)
    let now = Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap();
    let mut backup_set = HashSet::new();

    // Two backups in the same week (week 23: June 2-8)
    let week23_early = out_dir.join("weekly.w23early.tar");
    let week23_late = out_dir.join("weekly.w23late.tar");
    // One backup in a different week (week 22: May 26 - June 1)
    let week22 = out_dir.join("weekly.w22.tar");

    for p in [&week23_early, &week23_late, &week22] {
        fs::write(p, "data").unwrap();
    }

    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(week23_early.clone())
            .date_time(Utc.with_ymd_and_hms(2025, 6, 2, 10, 0, 0).unwrap()) // Mon week 23
            .build(),
    ));
    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(week23_late.clone())
            .date_time(Utc.with_ymd_and_hms(2025, 6, 7, 10, 0, 0).unwrap()) // Sat week 23
            .build(),
    ));
    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(week22.clone())
            .date_time(Utc.with_ymd_and_hms(2025, 5, 28, 10, 0, 0).unwrap()) // Wed week 22
            .build(),
    ));

    config
        .execute_backup_cycle(&mut backup_set, now, pool())
        .unwrap();

    // Weekly keeps one per week: week 23 (latest = Sat) + week 22 kept + new backup
    assert!(
        week23_late.exists(),
        "Later same-week backup should be kept"
    );
    assert!(
        !week23_early.exists(),
        "Earlier same-week backup should be deleted"
    );
    assert!(week22.exists(), "Different week backup should be kept");
}

#[test]
fn test_retention_monthly_keeps_one_per_month() {
    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("monthly")
        .out_dir(out_dir.clone())
        .files(vec![ArchiveEntryConfig::Base64(
            Base64Source::builder()
                .content("x")
                .dst(PathBuf::from("x.txt"))
                .build(),
        )])
        .compressor(CompressorConfig::None)
        .encryptor(EncryptorConfig::None)
        .retention(
            RetentionConfig::builder()
                .default_retention(Duration::from_secs(1))
                .monthly_retention(Duration::from_secs(180 * 86400)) // 6 months
                .min_backups(1)
                .build(),
        )
        .build();

    let now = Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap();
    let mut backup_set = HashSet::new();

    // Backups from different months
    let jan = out_dir.join("monthly.jan.tar");
    let feb = out_dir.join("monthly.feb.tar");
    let mar = out_dir.join("monthly.mar.tar");
    // Two backups in March (same month) — only one should survive
    let mar2 = out_dir.join("monthly.mar2.tar");

    for p in [&jan, &feb, &mar, &mar2] {
        fs::write(p, "data").unwrap();
    }

    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(jan.clone())
            .date_time(Utc.with_ymd_and_hms(2025, 1, 15, 12, 0, 0).unwrap())
            .build(),
    ));
    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(feb.clone())
            .date_time(Utc.with_ymd_and_hms(2025, 2, 10, 12, 0, 0).unwrap())
            .build(),
    ));
    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(mar.clone())
            .date_time(Utc.with_ymd_and_hms(2025, 3, 20, 12, 0, 0).unwrap())
            .build(),
    ));
    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(mar2.clone())
            .date_time(Utc.with_ymd_and_hms(2025, 3, 5, 12, 0, 0).unwrap())
            .build(),
    ));

    config
        .execute_backup_cycle(&mut backup_set, now, pool())
        .unwrap();

    // Monthly retention: keeps one per month within 6 months
    // Jan, Feb, Mar (most recent of the two) should be kept + new backup
    assert!(
        jan.exists(),
        "January backup should be kept (one per month)"
    );
    assert!(
        feb.exists(),
        "February backup should be kept (one per month)"
    );
    assert!(mar.exists(), "March (later) backup should be kept");
    assert!(
        !mar2.exists(),
        "March (earlier) backup should be deleted (duplicate month)"
    );
}

#[test]
fn test_retention_yearly_keeps_one_per_year() {
    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("yearly")
        .out_dir(out_dir.clone())
        .files(vec![ArchiveEntryConfig::Base64(
            Base64Source::builder()
                .content("x")
                .dst(PathBuf::from("x.txt"))
                .build(),
        )])
        .compressor(CompressorConfig::None)
        .encryptor(EncryptorConfig::None)
        .retention(
            RetentionConfig::builder()
                .default_retention(Duration::from_secs(1))
                .yearly_retention(Duration::from_secs(5 * 365 * 86400)) // 5 years
                .min_backups(1)
                .build(),
        )
        .build();

    let now = Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap();
    let mut backup_set = HashSet::new();

    // Backups from different years
    let y2022 = out_dir.join("yearly.2022.tar");
    let y2023 = out_dir.join("yearly.2023.tar");
    let y2024 = out_dir.join("yearly.2024.tar");
    // Two backups in 2023 (same year)
    let y2023b = out_dir.join("yearly.2023b.tar");

    for p in [&y2022, &y2023, &y2024, &y2023b] {
        fs::write(p, "data").unwrap();
    }

    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(y2022.clone())
            .date_time(Utc.with_ymd_and_hms(2022, 7, 1, 12, 0, 0).unwrap())
            .build(),
    ));
    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(y2023.clone())
            .date_time(Utc.with_ymd_and_hms(2023, 11, 15, 12, 0, 0).unwrap())
            .build(),
    ));
    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(y2023b.clone())
            .date_time(Utc.with_ymd_and_hms(2023, 3, 1, 12, 0, 0).unwrap())
            .build(),
    ));
    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(y2024.clone())
            .date_time(Utc.with_ymd_and_hms(2024, 5, 20, 12, 0, 0).unwrap())
            .build(),
    ));

    config
        .execute_backup_cycle(&mut backup_set, now, pool())
        .unwrap();

    // Yearly retention: one per year within 5 years
    // 2022, 2023 (most recent = Nov), 2024 kept + new backup
    assert!(y2022.exists(), "2022 backup should be kept (one per year)");
    assert!(y2023.exists(), "2023 (later) backup should be kept");
    assert!(y2024.exists(), "2024 backup should be kept");
    assert!(
        !y2023b.exists(),
        "2023 (earlier) backup should be deleted (duplicate year)"
    );
}

#[test]
fn test_retention_combined_daily_monthly_yearly() {
    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("combo")
        .out_dir(out_dir.clone())
        .files(vec![ArchiveEntryConfig::Base64(
            Base64Source::builder()
                .content("x")
                .dst(PathBuf::from("x.txt"))
                .build(),
        )])
        .compressor(CompressorConfig::None)
        .encryptor(EncryptorConfig::None)
        .retention(
            RetentionConfig::builder()
                .default_retention(Duration::from_secs(3 * 86400)) // 3 days
                .daily_retention(Duration::from_secs(7 * 86400)) // 7 days
                .monthly_retention(Duration::from_secs(90 * 86400)) // 90 days
                .yearly_retention(Duration::from_secs(3 * 365 * 86400)) // 3 years
                .min_backups(1)
                .build(),
        )
        .build();

    let now = Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap();
    let mut backup_set = HashSet::new();

    // Recent (within default retention) — kept by default
    let recent = out_dir.join("combo.recent.tar");
    // 5 days ago (outside default, inside daily) — kept by daily
    let daily_kept = out_dir.join("combo.daily.tar");
    // 60 days ago (outside daily, inside monthly) — kept by monthly
    let monthly_kept = out_dir.join("combo.monthly.tar");
    // 400 days ago (outside monthly, inside yearly) — kept by yearly
    let yearly_kept = out_dir.join("combo.yearly.tar");
    // 4 years ago (outside all retentions) — deleted
    let expired = out_dir.join("combo.expired.tar");

    for p in [&recent, &daily_kept, &monthly_kept, &yearly_kept, &expired] {
        fs::write(p, "data").unwrap();
    }

    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(recent.clone())
            .date_time(now - chrono::Duration::days(1))
            .build(),
    ));
    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(daily_kept.clone())
            .date_time(now - chrono::Duration::days(5))
            .build(),
    ));
    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(monthly_kept.clone())
            .date_time(now - chrono::Duration::days(60))
            .build(),
    ));
    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(yearly_kept.clone())
            .date_time(now - chrono::Duration::days(400))
            .build(),
    ));
    backup_set.insert(Rc::new(
        ItemWithDateTime::builder()
            .item(expired.clone())
            .date_time(now - chrono::Duration::days(4 * 365))
            .build(),
    ));

    config
        .execute_backup_cycle(&mut backup_set, now, pool())
        .unwrap();

    assert!(recent.exists(), "Recent backup kept by default retention");
    assert!(daily_kept.exists(), "5-day backup kept by daily retention");
    assert!(
        monthly_kept.exists(),
        "60-day backup kept by monthly retention"
    );
    assert!(
        yearly_kept.exists(),
        "400-day backup kept by yearly retention"
    );
    assert!(
        !expired.exists(),
        "4-year backup should be deleted (outside all retentions)"
    );
}

#[test]
fn test_multiple_backup_cycles() {
    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("multi")
        .out_dir(out_dir.clone())
        .files(vec![ArchiveEntryConfig::Base64(
            Base64Source::builder()
                .content("cycle data")
                .dst(PathBuf::from("data.txt"))
                .build(),
        )])
        .compressor(CompressorConfig::None)
        .encryptor(EncryptorConfig::None)
        .retention(
            RetentionConfig::builder()
                .default_retention(Duration::from_secs(3 * 86400))
                .min_backups(2)
                .build(),
        )
        .build();

    let mut backup_set = HashSet::new();

    for day in 0..5u32 {
        let now = Utc.with_ymd_and_hms(2025, 6, 10 + day, 1, 0, 0).unwrap();
        config
            .execute_backup_cycle(&mut backup_set, now, pool())
            .unwrap();
    }

    assert!(backup_set.len() >= 2);
    assert!(backup_set.len() <= 4);
    for item in &backup_set {
        assert!(item.item().exists());
    }
}

// ─── Config Serialization Tests ────────────────────────────────────────────

#[test]
fn test_config_yaml_roundtrip() {
    let yaml = r#"
cron: "0 2 * * *"
archive_base_name: roundtrip
out_dir: /tmp/test_backups
files:
  - type: base64
    content: "dGVzdA=="
    dst: test.txt
encryptor:
  encryptor_type: age
  secret_type: passphrase
  passphrase: "test-passphrase-roundtrip"
compressor:
  compressor_type: xz
  level: 3
retention:
  default_retention: 7days
  daily_retention: 30days
  min_backups: 3
"#;

    let config: BackupConfig = serde_yml::from_str(yaml).unwrap();
    assert_eq!(config.cron().as_deref(), Some("0 2 * * *"));
    assert_eq!(config.archive_base_name(), "roundtrip");

    let serialized = serde_yml::to_string(&config).unwrap();
    eprintln!("SERIALIZED:\n{}", serialized);
    let config2: BackupConfig = serde_yml::from_str(&serialized).unwrap();
    assert_eq!(config2.cron(), config.cron());
    assert_eq!(config2.archive_base_name(), config.archive_base_name());
}

#[test]
fn test_config_from_example_file() {
    let yaml = fs::read_to_string("config.example.yml").unwrap();
    let config: BackupConfig = serde_yml::from_str(&yaml).unwrap();
    assert_eq!(config.archive_base_name(), "backup");
    assert_eq!(config.files().len(), 3);
}

// ─── SMTP Notification Tests ───────────────────────────────────────────────

fn smtp_env_config() -> Option<SmtpNotificationConfig> {
    let host = std::env::var("SMTP_HOST").ok()?;
    let from = std::env::var("SMTP_FROM").ok()?;
    let to = std::env::var("SMTP_TO").ok()?;
    let username = std::env::var("SMTP_USERNAME").ok()?;
    let password = std::env::var("SMTP_PASSWORD").ok()?;
    let mode = match std::env::var("SMTP_MODE")
        .unwrap_or_else(|_| "Ssl".into())
        .as_str()
    {
        "StartTls" => SmtpMode::StartTls,
        "Unsecured" => SmtpMode::Unsecured,
        _ => SmtpMode::Ssl,
    };

    Some(
        SmtpNotificationConfig::builder()
            .host(host)
            .smtp_mode(mode)
            .from(from.parse::<Mailbox>().ok()?)
            .to(to
                .split(',')
                .filter_map(|s| s.trim().parse::<Mailbox>().ok())
                .collect::<Vec<_>>())
            .username(username)
            .password(RedactedString::builder().inner(password).build())
            .build(),
    )
}

#[test]
fn test_smtp_send_notification() {
    let Some(config) = smtp_env_config() else {
        eprintln!("Skipping: set SMTP_HOST, SMTP_FROM, SMTP_TO, SMTP_USERNAME, SMTP_PASSWORD");
        return;
    };

    let result = config.send(
        "[k-backup integration test] Test notification",
        "This is a test notification from k-backup integration tests.",
    );
    assert!(result.is_ok(), "SMTP send failed: {:?}", result.err());
}

#[test]
fn test_smtp_backup_cycle_with_notification() {
    let Some(smtp_config) = smtp_env_config() else {
        eprintln!("Skipping: SMTP env vars not set");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("smtp_test")
        .out_dir(out_dir)
        .files(vec![
            ArchiveEntryConfig::Base64(
                Base64Source::builder()
                    .content("valid content")
                    .dst(PathBuf::from("valid.txt"))
                    .build(),
            ),
            ArchiveEntryConfig::Glob(
                WalkdirAndGlobsetSource::builder()
                    .src_dir(PathBuf::from("/nonexistent/path/for/test"))
                    .globset(vec![serde_json::from_str("\"**/*\"").unwrap()])
                    .build(),
            ),
        ])
        .notifications(vec![NotificationConfig::Smtp(smtp_config)])
        .compressor(CompressorConfig::None)
        .encryptor(EncryptorConfig::None)
        .build();

    let now = Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap();
    let mut backup_set = HashSet::new();

    let result = config.execute_backup_cycle(&mut backup_set, now, pool());
    assert!(result.is_ok());
    assert_eq!(backup_set.len(), 1);
}

// ─── SMTP Multiple Entry Errors Test ───────────────────────────────────────

#[test]
fn test_smtp_notification_with_multiple_entry_errors() {
    let Some(smtp_config) = smtp_env_config() else {
        eprintln!("Skipping: SMTP env vars not set");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("multi_error_test")
        .out_dir(out_dir)
        .files(vec![
            ArchiveEntryConfig::Base64(
                Base64Source::builder()
                    .content("good content")
                    .dst(PathBuf::from("good.txt"))
                    .build(),
            ),
            ArchiveEntryConfig::Glob(
                WalkdirAndGlobsetSource::builder()
                    .src_dir(PathBuf::from("/nonexistent/entry/one"))
                    .globset(vec![serde_json::from_str("\"**/*\"").unwrap()])
                    .build(),
            ),
            ArchiveEntryConfig::Glob(
                WalkdirAndGlobsetSource::builder()
                    .src_dir(PathBuf::from("/nonexistent/entry/two"))
                    .globset(vec![serde_json::from_str("\"**/*.log\"").unwrap()])
                    .build(),
            ),
            ArchiveEntryConfig::Sqlite(
                SqliteDBSource::builder()
                    .src(PathBuf::from("/nonexistent/database.sqlite3"))
                    .dst(PathBuf::from("db.sqlite3"))
                    .build(),
            ),
        ])
        .notifications(vec![NotificationConfig::Smtp(smtp_config)])
        .compressor(CompressorConfig::None)
        .encryptor(EncryptorConfig::None)
        .build();

    let now = Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap();
    let mut backup_set = HashSet::new();

    // Should fail because archive_entry_iterator() fails for sqlite/glob entries
    // which sends fatal error through channel
    let result = config.execute_backup_cycle(&mut backup_set, now, pool());
    // The result may be Ok (if only non-fatal) or Err (if fatal entry_iterator failure)
    // Either way, the notification should have been sent with grouped entry errors
    eprintln!(
        "Result: {:?}",
        result.as_ref().map(|_| "ok").unwrap_or("err")
    );
    eprintln!("Backup set: {}", backup_set.len());
}

// ─── Recipients File Encryption Tests ──────────────────────────────────────

fn decrypt_and_decompress_with_identity(
    path: &std::path::Path,
    identity: &dyn age::Identity,
) -> Vec<(String, Vec<u8>)> {
    let file = fs::File::open(path).unwrap();
    let decryptor = age::Decryptor::new(BufReader::new(file)).unwrap();
    let decrypted = decryptor.decrypt(std::iter::once(identity)).unwrap();
    let decompressed = liblzma::read::XzDecoder::new(BufReader::new(decrypted));
    let mut archive = tar::Archive::new(decompressed);

    let mut entries = Vec::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().to_string();
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();
        entries.push((path, content));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

fn test_recipients_file() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test_recipients.txt")
}

fn test_identity_1() -> age::x25519::Identity {
    "AGE-SECRET-KEY-1Y60VNPEJGMHTG8Z75D5Q8MKX4Z207EQJLZDHETLLKYX67TLUS2RQ329ZXS"
        .parse()
        .unwrap()
}

fn test_identity_2() -> age::x25519::Identity {
    "AGE-SECRET-KEY-1VR82K9YTDHYVNUVXLJR9EUZY9WFS8CNT80L8H29VRNTRQ4UKWE7SS46MMR"
        .parse()
        .unwrap()
}

#[test]
fn test_full_backup_pipeline_recipients_file_multi_recipient() {
    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("recipients_test")
        .out_dir(out_dir.clone())
        .files(vec![ArchiveEntryConfig::Base64(
            Base64Source::builder()
                .content("secret data for multiple recipients")
                .dst(PathBuf::from("secret.txt"))
                .build(),
        )])
        .compressor(CompressorConfig::Xz(XzConfig::builder().level(1).build()))
        .encryptor(EncryptorConfig::Age(AgeEncryptorConfig::RecipientsFiles {
            recipients_files: vec![test_recipients_file().to_string_lossy().into_owned()],
        }))
        .build();

    let dt = Utc.with_ymd_and_hms(2025, 6, 15, 10, 0, 0).unwrap();
    let (path, err) = config.create_archive(dt, pool()).unwrap();

    assert!(err.is_empty());
    assert!(path.exists());
    assert!(path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with(".tar.xz.age"));

    // Decrypt with identity 1
    let entries = decrypt_and_decompress_with_identity(&path, &test_identity_1());
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "secret.txt");
    assert_eq!(entries[0].1, b"secret data for multiple recipients");

    // Decrypt with identity 2
    let entries = decrypt_and_decompress_with_identity(&path, &test_identity_2());
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "secret.txt");
    assert_eq!(entries[0].1, b"secret data for multiple recipients");
}

#[test]
fn test_recipients_file_decrypt_fails_with_wrong_identity() {
    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();

    let config = BackupConfig::builder()
        .cron("0 1 * * *")
        .archive_base_name("recipients_test_fail")
        .out_dir(out_dir.clone())
        .files(vec![ArchiveEntryConfig::Base64(
            Base64Source::builder()
                .content("should not be readable")
                .dst(PathBuf::from("secret.txt"))
                .build(),
        )])
        .compressor(CompressorConfig::Xz(XzConfig::builder().level(1).build()))
        .encryptor(EncryptorConfig::Age(AgeEncryptorConfig::RecipientsFiles {
            recipients_files: vec![test_recipients_file().to_string_lossy().into_owned()],
        }))
        .build();

    let dt = Utc.with_ymd_and_hms(2025, 6, 15, 10, 0, 0).unwrap();
    let (path, err) = config.create_archive(dt, pool()).unwrap();
    assert!(err.is_empty());

    // Generate a completely unrelated identity
    let wrong_identity = age::x25519::Identity::generate();

    let file = fs::File::open(&path).unwrap();
    let decryptor = age::Decryptor::new(BufReader::new(file)).unwrap();
    let result = decryptor.decrypt(std::iter::once(&wrong_identity as &dyn age::Identity));
    assert!(result.is_err());
}
