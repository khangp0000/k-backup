//! Configuration types for k-backup. Deserialized from YAML.

use crate::error::{ConfigError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::{self, Debug, Formatter};
use std::path::PathBuf;
use std::result;
use std::sync::Arc;
use std::time::Duration;
use zeroize::{Zeroize, ZeroizeOnDrop};

// ─── Top-level Config ─────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupConfig {
    pub cron: Option<String>,
    pub archive_base_name: String,
    pub out_dir: PathBuf,
    /// Directory for temp files during archive creation. Default: system temp dir.
    #[serde(default)]
    pub temp_dir: Option<PathBuf>,
    pub files: Vec<ArchiveEntryConfig>,
    #[serde(default)]
    pub notifications: Vec<NotificationConfig>,
    pub compressor: CompressorConfig,
    pub encryptor: EncryptorConfig,
    pub retention: Option<RetentionConfig>,
}

impl BackupConfig {
    pub fn validate(&self) -> Result<()> {
        let mut errors = Vec::new();

        if self.archive_base_name.is_empty() {
            errors.push("archive_base_name must not be empty".into());
        }
        if self.archive_base_name.contains('/') || self.archive_base_name.contains('\0') {
            errors.push("archive_base_name must not contain '/' or null bytes".into());
        }
        if !self.out_dir.exists() {
            errors.push(format!("out_dir does not exist: {:?}", self.out_dir));
        } else if std::fs::metadata(&self.out_dir)
            .map(|m| m.permissions().readonly())
            .unwrap_or(true)
        {
            errors.push(format!("out_dir is not writable: {:?}", self.out_dir));
        }
        if let Some(ref temp_dir) = self.temp_dir {
            if !temp_dir.exists() {
                errors.push(format!("temp_dir does not exist: {:?}", temp_dir));
            }
        }
        if self.files.is_empty() {
            errors.push("files must contain at least one entry".into());
        }
        if let Some(ref cron) = self.cron {
            if cron_parser::parse(cron, &Utc::now()).is_err() {
                errors.push(format!("Invalid cron expression: {}", cron));
            }
        }

        self.compressor.validate(&mut errors);
        self.encryptor.validate(&mut errors);
        for (i, f) in self.files.iter().enumerate() {
            f.validate(i, &mut errors);
        }
        for (i, n) in self.notifications.iter().enumerate() {
            n.validate(i, &mut errors);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ConfigError::Validation(errors).into())
        }
    }

    pub fn file_ext(&self) -> String {
        let mut parts = vec!["tar"];
        if let Some(ext) = self.compressor.file_ext() {
            parts.push(ext);
        }
        if let Some(ext) = self.encryptor.file_ext() {
            parts.push(ext);
        }
        parts.join(".")
    }
}

// ─── Archive Entry Config ─────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ArchiveEntryConfig {
    Sqlite(SqliteSourceConfig),
    Glob(GlobSourceConfig),
    Base64(Base64SourceConfig),
}

impl ArchiveEntryConfig {
    pub fn is_required(&self) -> bool {
        match self {
            Self::Sqlite(c) => c.required,
            Self::Glob(c) => c.required,
            Self::Base64(_) => true, // always succeeds, effectively always "required"
        }
    }

    fn validate(&self, index: usize, errors: &mut Vec<String>) {
        match self {
            Self::Glob(c) => {
                if c.globset.is_empty() {
                    errors.push(format!("files[{}]: globset must not be empty", index));
                }
                if c.symlink_mode == SymlinkMode::Follow && c.max_depth == 0 {
                    tracing::warn!(
                        "files[{}]: symlink_mode=follow without max_depth may traverse symlink loops",
                        index
                    );
                }
            }
            Self::Sqlite(c) => {
                if c.dst.as_os_str().is_empty() {
                    errors.push(format!("files[{}]: dst must not be empty", index));
                }
            }
            Self::Base64(c) => {
                if c.dst.as_os_str().is_empty() {
                    errors.push(format!("files[{}]: dst must not be empty", index));
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SqliteSourceConfig {
    pub src: PathBuf,
    pub dst: PathBuf,
    #[serde(default = "default_true")]
    pub required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobSourceConfig {
    pub src_dir: PathBuf,
    #[serde(default)]
    pub dst_dir: Option<String>,
    pub globset: CompiledGlobSet,
    #[serde(default = "default_symlink_mode")]
    pub symlink_mode: SymlinkMode,
    #[serde(default)]
    pub max_depth: usize,
    #[serde(default = "default_true")]
    pub required: bool,
}

/// A validated glob set that compiles patterns at deserialization time.
/// Accepts a single string or a list of strings in YAML/JSON.
#[derive(Clone)]
pub struct CompiledGlobSet {
    patterns: Vec<String>,
    compiled: globset::GlobSet,
}

impl CompiledGlobSet {
    pub fn new(patterns: Vec<String>) -> std::result::Result<Self, globset::Error> {
        let mut builder = globset::GlobSetBuilder::new();
        for pattern in &patterns {
            builder.add(globset::GlobBuilder::new(pattern).build()?);
        }
        Ok(Self {
            compiled: builder.build()?,
            patterns,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    pub fn is_match(&self, path: &std::path::Path) -> bool {
        self.compiled.is_match(path)
    }
}

impl fmt::Debug for CompiledGlobSet {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(&self.patterns).finish()
    }
}

impl Serialize for CompiledGlobSet {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> result::Result<S::Ok, S::Error> {
        self.patterns.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CompiledGlobSet {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> result::Result<Self, D::Error> {
        struct GlobSetVisitor;

        impl<'de> serde::de::Visitor<'de> for GlobSetVisitor {
            type Value = CompiledGlobSet;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a glob pattern string or list of glob patterns")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> result::Result<Self::Value, E> {
                CompiledGlobSet::new(vec![v.to_string()]).map_err(E::custom)
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> result::Result<Self::Value, A::Error> {
                let mut patterns = Vec::new();
                while let Some(s) = seq.next_element::<String>()? {
                    patterns.push(s);
                }
                CompiledGlobSet::new(patterns).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_any(GlobSetVisitor)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Base64SourceConfig {
    pub content: Base64Bytes,
    pub dst: PathBuf,
}

/// Raw bytes decoded from base64 at deserialization. Shared via Arc to avoid cloning.
#[derive(Clone)]
pub struct Base64Bytes(Arc<[u8]>);

impl Base64Bytes {
    #[cfg(test)]
    pub fn new(data: Vec<u8>) -> Self {
        Self(data.into())
    }

    pub fn arc(&self) -> &Arc<[u8]> {
        &self.0
    }
}

impl fmt::Debug for Base64Bytes {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Base64Bytes({} bytes)", self.0.len())
    }
}

impl Serialize for Base64Bytes {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> result::Result<S::Ok, S::Error> {
        use base64::Engine;
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(&*self.0))
    }
}

impl<'de> Deserialize<'de> for Base64Bytes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> result::Result<Self, D::Error> {
        use base64::Engine;
        let s = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(&s)
            .map(|v| Self(v.into()))
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SymlinkMode {
    Follow,
    Preserve,
    Skip,
}

fn default_symlink_mode() -> SymlinkMode {
    SymlinkMode::Follow
}

// ─── Compressor Config ────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "compressor_type", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum CompressorConfig {
    None,
    Xz {
        #[serde(default = "default_xz_level")]
        level: u32,
        #[serde(default)]
        thread: Option<usize>,
    },
}

impl CompressorConfig {
    pub fn file_ext(&self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Xz { .. } => Some("xz"),
        }
    }

    fn validate(&self, errors: &mut Vec<String>) {
        if let Self::Xz { level, .. } = self {
            if *level > 9 {
                errors.push(format!("compressor.level must be 0-9, got {}", level));
            }
        }
    }
}

fn default_xz_level() -> u32 {
    6
}

// ─── Encryptor Config ─────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "encryptor_type", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum EncryptorConfig {
    None,
    Age(AgeConfig),
}

impl EncryptorConfig {
    pub fn file_ext(&self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Age(_) => Some("age"),
        }
    }

    fn validate(&self, errors: &mut Vec<String>) {
        if let Self::Age(age) = self {
            age.validate(errors);
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "secret_type", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum AgeConfig {
    Passphrase { passphrase: RedactedString },
    RecipientsFiles { recipients_files: Vec<PathBuf> },
}

impl AgeConfig {
    fn validate(&self, errors: &mut Vec<String>) {
        match self {
            Self::Passphrase { passphrase } => {
                if passphrase.inner.len() < 8 {
                    errors.push("encryptor.passphrase must be at least 8 characters".into());
                }
            }
            Self::RecipientsFiles { recipients_files } => {
                if recipients_files.is_empty() {
                    errors.push("encryptor.recipients_files must not be empty".into());
                }
            }
        }
    }
}

impl Debug for AgeConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Passphrase { .. } => f
                .debug_struct("Passphrase")
                .field("passphrase", &"***")
                .finish(),
            Self::RecipientsFiles { recipients_files } => f
                .debug_struct("RecipientsFiles")
                .field("recipients_files", recipients_files)
                .finish(),
        }
    }
}

// ─── Redacted String ──────────────────────────────────────────────────────

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct RedactedString {
    inner: String,
}

impl RedactedString {
    pub fn new(s: impl Into<String>) -> Self {
        Self { inner: s.into() }
    }

    pub fn inner(&self) -> &str {
        &self.inner
    }
}

impl Debug for RedactedString {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "\"###REDACTED###\"")
    }
}

impl Serialize for RedactedString {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> result::Result<S::Ok, S::Error> {
        serializer.serialize_str("###REDACTED_PASSPHRASE###")
    }
}

impl<'de> Deserialize<'de> for RedactedString {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> result::Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

// ─── Retention Config ─────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionConfig {
    #[serde(with = "humantime_serde")]
    pub default_retention: Duration,
    #[serde(default, with = "humantime_serde::option")]
    pub daily_retention: Option<Duration>,
    #[serde(default, with = "humantime_serde::option")]
    pub weekly_retention: Option<Duration>,
    #[serde(default, with = "humantime_serde::option")]
    pub monthly_retention: Option<Duration>,
    #[serde(default, with = "humantime_serde::option")]
    pub yearly_retention: Option<Duration>,
    #[serde(default = "default_min_backups")]
    pub min_backups: usize,
}

fn default_min_backups() -> usize {
    3
}

// ─── Notification Config ──────────────────────────────────────────────────

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

impl NotificationConfig {
    pub fn display_name(&self, index: usize) -> String {
        self.name.clone().unwrap_or_else(|| {
            let t = match &self.target {
                NotificationTarget::Smtp(_) => "smtp",
                NotificationTarget::Command(_) => "command",
            };
            format!("{}-{}", t, index)
        })
    }

    fn validate(&self, index: usize, errors: &mut Vec<String>) {
        match &self.target {
            NotificationTarget::Smtp(s) => {
                if s.to.is_empty() {
                    errors.push(format!(
                        "notifications[{}]: to list must not be empty",
                        index
                    ));
                }
            }
            NotificationTarget::Command(c) => {
                if c.command.is_empty() {
                    errors.push(format!(
                        "notifications[{}]: command must not be empty",
                        index
                    ));
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationTarget {
    Smtp(SmtpConfig),
    Command(CommandConfig),
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OnFailure {
    #[default]
    Continue,
    Skip,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    BackupCycleStart,
    Success,
    NonFatalError,
    FatalError,
}

fn default_events() -> Vec<EventType> {
    vec![EventType::NonFatalError, EventType::FatalError]
}

// ─── SMTP Config ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SmtpConfig {
    pub host: String,
    pub smtp_mode: SmtpMode,
    pub from: String,
    pub to: Vec<String>,
    pub username: String,
    pub password: RedactedString,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SmtpMode {
    Unsecured,
    Ssl,
    StartTls,
}

// ─── Command Config ───────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandConfig {
    pub command: Vec<String>,
    #[serde(default = "default_true")]
    pub stdin_json: bool,
    #[serde(default)]
    pub env_inherit_mode: EnvInheritMode,
    #[serde(default)]
    pub env_inherit_allow: Vec<String>,
    #[serde(default)]
    pub env_inherit_deny: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "default_timeout", with = "humantime_serde")]
    pub timeout: Duration,
    #[serde(default = "default_max_output_size")]
    pub max_output_size: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnvInheritMode {
    All,
    #[default]
    None,
}

fn default_timeout() -> Duration {
    Duration::from_secs(30)
}

fn default_max_output_size() -> usize {
    65536 // 64KB
}

// ─── Helpers ──────────────────────────────────────────────────────────────

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_config(tmp: &std::path::Path) -> BackupConfig {
        BackupConfig {
            cron: None,
            archive_base_name: "test".into(),
            out_dir: tmp.to_path_buf(),
            temp_dir: None,
            files: vec![ArchiveEntryConfig::Base64(Base64SourceConfig {
                content: Base64Bytes::new(b"hello".to_vec()),
                dst: "hello.txt".into(),
            })],
            notifications: vec![],
            compressor: CompressorConfig::None,
            encryptor: EncryptorConfig::None,
            retention: None,
        }
    }

    #[test]
    fn validate_empty_archive_base_name() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = minimal_config(tmp.path());
        cfg.archive_base_name = "".into();
        let err = cfg.validate().unwrap_err();
        assert!(err
            .to_string()
            .contains("archive_base_name must not be empty"));
    }

    #[test]
    fn validate_invalid_cron() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = minimal_config(tmp.path());
        cfg.cron = Some("not a cron".into());
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("Invalid cron expression"));
    }

    #[test]
    fn validate_empty_files() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = minimal_config(tmp.path());
        cfg.files = vec![];
        let err = cfg.validate().unwrap_err();
        assert!(err
            .to_string()
            .contains("files must contain at least one entry"));
    }

    #[test]
    fn validate_short_passphrase() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = minimal_config(tmp.path());
        cfg.encryptor = EncryptorConfig::Age(AgeConfig::Passphrase {
            passphrase: RedactedString::new("short"),
        });
        let err = cfg.validate().unwrap_err();
        assert!(err
            .to_string()
            .contains("passphrase must be at least 8 characters"));
    }

    #[test]
    fn redacted_string_serializes_as_redacted() {
        let rs = RedactedString::new("secret");
        let json = serde_json::to_string(&rs).unwrap();
        assert_eq!(json, "\"###REDACTED_PASSPHRASE###\"");
    }

    #[test]
    fn redacted_string_debug_shows_stars() {
        let rs = RedactedString::new("secret");
        let debug = format!("{:?}", rs);
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("secret"));
    }

    #[test]
    fn full_config_yaml_roundtrip() {
        let yaml = r#"
archive_base_name: backup
out_dir: /tmp
files:
  - type: base64
    content: "aGVsbG8="
    dst: hello.txt
notifications: []
compressor:
  compressor_type: xz
  level: 6
encryptor:
  encryptor_type: age
  secret_type: passphrase
  passphrase: "my-secure-passphrase"
retention:
  default_retention: 7days
  min_backups: 3
"#;
        let config: BackupConfig = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(config.archive_base_name, "backup");
        assert_eq!(config.files.len(), 1);
        match &config.encryptor {
            EncryptorConfig::Age(AgeConfig::Passphrase { passphrase }) => {
                assert_eq!(passphrase.inner(), "my-secure-passphrase");
            }
            _ => panic!("expected passphrase encryptor"),
        }
        match &config.compressor {
            CompressorConfig::Xz { level, .. } => assert_eq!(*level, 6),
            _ => panic!("expected xz compressor"),
        }
        let retention = config.retention.unwrap();
        assert_eq!(retention.min_backups, 3);
    }
}
