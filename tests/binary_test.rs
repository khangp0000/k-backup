//! End-to-end binary tests for k_backup.
//!
//! By default tests the current crate binary. Set K_BACKUP_BIN env var to test another binary:
//!   K_BACKUP_BIN=/path/to/k-backup/target/debug/k_backup cargo test --test binary_test

use assert_cmd::Command;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn cmd() -> Command {
    match std::env::var("K_BACKUP_BIN") {
        Ok(path) => Command::new(path),
        Err(_) => Command::cargo_bin("k_backup").unwrap(),
    }
}

fn write_config(dir: &Path, yaml: &str) -> PathBuf {
    let path = dir.join("config.yml");
    fs::write(&path, yaml).unwrap();
    path
}

fn create_source_dir(dir: &Path) -> PathBuf {
    let src = dir.join("source");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("hello.txt"), "hello world").unwrap();
    fs::write(src.join("data.json"), r#"{"key":"value"}"#).unwrap();
    fs::write(src.join("sub/nested.txt"), "nested content").unwrap();
    fs::write(src.join("ignore.log"), "ignored").unwrap();
    src
}

fn create_sqlite_db(path: &Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT);
         INSERT INTO test VALUES (1, 'alice');
         INSERT INTO test VALUES (2, 'bob');",
    )
    .unwrap();
}

fn list_backup_files(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect()
}

fn decrypt_age(path: &Path, passphrase: &str) -> Vec<u8> {
    let file = fs::File::open(path).unwrap();
    let decryptor = age::Decryptor::new(std::io::BufReader::new(file)).unwrap();
    let mut identity = age::scrypt::Identity::new(age::secrecy::SecretString::new(
        passphrase.to_string().into(),
    ));
    identity.set_max_work_factor(22);
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .unwrap();
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).unwrap();
    buf
}

fn decompress_xz(data: &[u8]) -> Vec<u8> {
    let mut decoder = liblzma::read::XzDecoder::new(std::io::BufReader::new(data));
    let mut buf = Vec::new();
    decoder.read_to_end(&mut buf).unwrap();
    buf
}

fn tar_entries(data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = tar::Archive::new(data);
    archive
        .entries()
        .unwrap()
        .map(|e| {
            let mut entry = e.unwrap();
            let path = entry.path().unwrap().to_string_lossy().to_string();
            let mut content = Vec::new();
            entry.read_to_end(&mut content).unwrap();
            (path, content)
        })
        .collect()
}

// ─── Core Backup Tests ─────────────────────────────────────────────────────

#[test]
fn test_base64_source() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: test
out_dir: {}
files:
  - type: base64
    content: "aGVsbG8gd29ybGQ="
    dst: hello.txt
compressor:
  compressor_type: none
encryptor:
  encryptor_type: none
"#,
            out.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().success();

    let files = list_backup_files(&out);
    assert_eq!(files.len(), 1);
    let entries = tar_entries(&fs::read(&files[0]).unwrap());
    assert_eq!(entries[0].0, "hello.txt");
    assert_eq!(entries[0].1, b"hello world");
}

#[test]
fn test_glob_source() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let src = create_source_dir(tmp.path());
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: g
out_dir: {}
files:
  - type: glob
    src_dir: {}
    globset: ["**/*.txt", "**/*.json"]
compressor:
  compressor_type: none
encryptor:
  encryptor_type: none
"#,
            out.display(),
            src.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().success();

    let files = list_backup_files(&out);
    let entries = tar_entries(&fs::read(&files[0]).unwrap());
    let paths: Vec<&str> = entries.iter().map(|e| e.0.as_str()).collect();
    assert!(paths.contains(&"hello.txt"));
    assert!(paths.contains(&"data.json"));
    assert!(paths.contains(&"sub/nested.txt"));
    assert!(!paths.iter().any(|p| p.ends_with(".log")));
}

#[test]
fn test_sqlite_source() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let db = tmp.path().join("test.db");
    create_sqlite_db(&db);
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: s
out_dir: {}
files:
  - type: sqlite
    src: {}
    dst: backup.db
compressor:
  compressor_type: none
encryptor:
  encryptor_type: none
"#,
            out.display(),
            db.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().success();

    let files = list_backup_files(&out);
    let entries = tar_entries(&fs::read(&files[0]).unwrap());
    assert_eq!(entries[0].0, "backup.db");
    assert!(entries[0].1.starts_with(b"SQLite format 3"));
}

#[test]
fn test_mixed_sources() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let src = create_source_dir(tmp.path());
    let db = tmp.path().join("test.db");
    create_sqlite_db(&db);
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: m
out_dir: {}
files:
  - type: base64
    content: "dGVzdA=="
    dst: b64.txt
  - type: glob
    src_dir: {}
    globset: ["**/*.txt"]
  - type: sqlite
    src: {}
    dst: data.db
compressor:
  compressor_type: none
encryptor:
  encryptor_type: none
"#,
            out.display(),
            src.display(),
            db.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().success();

    let files = list_backup_files(&out);
    let entries = tar_entries(&fs::read(&files[0]).unwrap());
    let paths: Vec<&str> = entries.iter().map(|e| e.0.as_str()).collect();
    assert!(paths.contains(&"b64.txt"));
    assert!(paths.contains(&"hello.txt"));
    assert!(paths.contains(&"data.db"));
}

// ─── Compression + Encryption Tests ───────────────────────────────────────

#[test]
fn test_xz_compression() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: x
out_dir: {}
files:
  - type: base64
    content: "aGVsbG8="
    dst: f.txt
compressor:
  compressor_type: xz
  level: 3
encryptor:
  encryptor_type: none
"#,
            out.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().success();

    let files = list_backup_files(&out);
    assert!(files[0].to_str().unwrap().ends_with(".tar.xz"));
    let entries = tar_entries(&decompress_xz(&fs::read(&files[0]).unwrap()));
    assert_eq!(entries[0].1, b"hello");
}

#[test]
fn test_age_passphrase_encryption() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: e
out_dir: {}
files:
  - type: base64
    content: "c2VjcmV0"
    dst: secret.txt
compressor:
  compressor_type: none
encryptor:
  encryptor_type: age
  secret_type: passphrase
  passphrase: "test-pass-phrase"
"#,
            out.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().success();

    let files = list_backup_files(&out);
    assert!(files[0].to_str().unwrap().ends_with(".tar.age"));
    let entries = tar_entries(&decrypt_age(&files[0], "test-pass-phrase"));
    assert_eq!(entries[0].1, b"secret");
}

#[test]
fn test_full_pipeline() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: full
out_dir: {}
files:
  - type: base64
    content: "ZnVsbA=="
    dst: f.txt
compressor:
  compressor_type: xz
  level: 6
encryptor:
  encryptor_type: age
  secret_type: passphrase
  passphrase: "full-pipeline"
"#,
            out.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().success();

    let files = list_backup_files(&out);
    assert!(files[0].to_str().unwrap().ends_with(".tar.xz.age"));
    let entries = tar_entries(&decompress_xz(&decrypt_age(&files[0], "full-pipeline")));
    assert_eq!(entries[0].1, b"full");
}

#[test]
fn test_recipients_file() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let identity = age::x25519::Identity::generate();
    let recip_file = tmp.path().join("recipients.txt");
    fs::write(&recip_file, identity.to_public().to_string()).unwrap();
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: r
out_dir: {}
files:
  - type: base64
    content: "cmVjaXA="
    dst: r.txt
compressor:
  compressor_type: none
encryptor:
  encryptor_type: age
  secret_type: recipients_files
  recipients_files: ["{}"]
"#,
            out.display(),
            recip_file.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().success();

    let files = list_backup_files(&out);
    let file = fs::File::open(&files[0]).unwrap();
    let decryptor = age::Decryptor::new(std::io::BufReader::new(file)).unwrap();
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .unwrap();
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).unwrap();
    let entries = tar_entries(&buf);
    assert_eq!(entries[0].1, b"recip");
}

// ─── Retention Tests ──────────────────────────────────────────────────────

#[test]
fn test_retention_deletes_old() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let old = out.join("ret.2020-01-01T01h00m00s_0000.tar");
    fs::write(&old, "old").unwrap();
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: ret
out_dir: {}
files:
  - type: base64
    content: "bmV3"
    dst: n.txt
compressor:
  compressor_type: none
encryptor:
  encryptor_type: none
retention:
  default_retention: 1day
  min_backups: 1
"#,
            out.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().success();
    assert!(!old.exists());
    assert_eq!(list_backup_files(&out).len(), 1);
}

#[test]
fn test_retention_min_backups() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let old1 = out.join("min.2020-01-01T01h00m00s_0000.tar");
    let old2 = out.join("min.2020-01-02T01h00m00s_0000.tar");
    fs::write(&old1, "1").unwrap();
    fs::write(&old2, "2").unwrap();
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: min
out_dir: {}
files:
  - type: base64
    content: "bmV3"
    dst: n.txt
compressor:
  compressor_type: none
encryptor:
  encryptor_type: none
retention:
  default_retention: 1s
  min_backups: 5
"#,
            out.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().success();
    assert!(old1.exists());
    assert!(old2.exists());
    assert_eq!(list_backup_files(&out).len(), 3);
}

// ─── Notification Tests ───────────────────────────────────────────────────

#[test]
fn test_command_notification_success() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let marker = tmp.path().join("hook.json");
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: n
out_dir: {out}
files:
  - type: base64
    content: "b2s="
    dst: ok.txt
compressor:
  compressor_type: none
encryptor:
  encryptor_type: none
notifications:
  - type: command
    events: [success]
    command: ["bash", "-c", "cat > {marker}"]
    stdin_json: true
"#,
            out = out.display(),
            marker = marker.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().success();
    assert!(marker.exists());
    let content = fs::read_to_string(&marker).unwrap();
    assert!(content.contains("\"type\":\"success\""));
}

#[test]
fn test_command_notification_event_filtering() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let marker = tmp.path().join("nope");
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: f
out_dir: {out}
files:
  - type: base64
    content: "b2s="
    dst: ok.txt
compressor:
  compressor_type: none
encryptor:
  encryptor_type: none
notifications:
  - type: command
    events: [fatal_error]
    command: ["touch", "{marker}"]
    stdin_json: false
"#,
            out = out.display(),
            marker = marker.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().success();
    assert!(!marker.exists());
}

#[test]
fn test_on_failure_continue() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: c
out_dir: {}
files:
  - type: base64
    content: "b2s="
    dst: ok.txt
compressor:
  compressor_type: none
encryptor:
  encryptor_type: none
notifications:
  - type: command
    events: [success]
    on_failure: continue
    command: ["false"]
    stdin_json: false
"#,
            out.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().success();
    assert_eq!(list_backup_files(&out).len(), 1);
}

#[test]
fn test_on_failure_error() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: e
out_dir: {}
files:
  - type: base64
    content: "b2s="
    dst: ok.txt
compressor:
  compressor_type: none
encryptor:
  encryptor_type: none
notifications:
  - type: command
    events: [success]
    on_failure: error
    command: ["false"]
    stdin_json: false
"#,
            out.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().failure();
}

#[test]
fn test_on_failure_skip() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: s
out_dir: {}
files:
  - type: base64
    content: "b2s="
    dst: ok.txt
compressor:
  compressor_type: none
encryptor:
  encryptor_type: none
notifications:
  - type: command
    events: [backup_cycle_start]
    on_failure: skip
    command: ["false"]
    stdin_json: false
"#,
            out.display()
        ),
    );

    // Skip is graceful — exits 0 but no backup produced
    cmd().arg("-c").arg(&cfg).arg("--once").assert().success();
    assert_eq!(list_backup_files(&out).len(), 0);
}

#[test]
fn test_command_timeout() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: t
out_dir: {}
files:
  - type: base64
    content: "b2s="
    dst: ok.txt
compressor:
  compressor_type: none
encryptor:
  encryptor_type: none
notifications:
  - type: command
    events: [success]
    on_failure: error
    command: ["sleep", "60"]
    stdin_json: false
    timeout: 200ms
"#,
            out.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().failure();
}

#[test]
fn test_command_env() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let envf = tmp.path().join("env.txt");
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: ev
out_dir: {out}
files:
  - type: base64
    content: "b2s="
    dst: ok.txt
compressor:
  compressor_type: none
encryptor:
  encryptor_type: none
notifications:
  - type: command
    events: [success]
    command: ["bash", "-c", "env > {envf}"]
    stdin_json: false
    env_inherit_mode: none
    env:
      MY_VAR: "hello"
"#,
            out = out.display(),
            envf = envf.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().success();
    let content = fs::read_to_string(&envf).unwrap();
    assert!(content.contains("MY_VAR=hello"));
}

// ─── Error Cases ──────────────────────────────────────────────────────────

#[test]
fn test_invalid_config() {
    let tmp = TempDir::new().unwrap();
    let cfg = write_config(tmp.path(), "invalid: [[[");
    cmd().arg("-c").arg(&cfg).arg("--once").assert().failure();
}

#[test]
fn test_missing_config() {
    cmd()
        .arg("-c")
        .arg("/nonexistent.yml")
        .arg("--once")
        .assert()
        .failure();
}

#[test]
fn test_no_cron_runs_once() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: nc
out_dir: {}
files:
  - type: base64
    content: "b25jZQ=="
    dst: once.txt
compressor:
  compressor_type: none
encryptor:
  encryptor_type: none
"#,
            out.display()
        ),
    );

    // No --once flag, no cron → should run once and exit
    cmd().arg("-c").arg(&cfg).assert().success();
    assert_eq!(list_backup_files(&out).len(), 1);
}

#[test]
fn test_output_filename_format() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let cfg = write_config(
        tmp.path(),
        &format!(
            r#"
archive_base_name: fmt
out_dir: {}
files:
  - type: base64
    content: "dGVzdA=="
    dst: t.txt
compressor:
  compressor_type: xz
encryptor:
  encryptor_type: age
  secret_type: passphrase
  passphrase: "fmt-test-pass1"
"#,
            out.display()
        ),
    );

    cmd().arg("-c").arg(&cfg).arg("--once").assert().success();

    let files = list_backup_files(&out);
    let name = files[0].file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("fmt."));
    assert!(name.ends_with(".tar.xz.age"));
    let middle = &name["fmt.".len()..name.len() - ".tar.xz.age".len()];
    assert!(middle.contains('T'));
    assert!(middle.contains('h'));
    assert!(middle.contains('m'));
}
