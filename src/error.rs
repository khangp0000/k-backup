//! Error types for k-backup. All error structs are boxed (pointer-sized).

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

// ─── Top-level Error ──────────────────────────────────────────────────────

#[derive(Debug)]
pub struct Error(Box<ErrorKind>);

#[derive(Debug, thiserror::Error)]
pub enum ErrorKind {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    #[error(transparent)]
    Encrypt(#[from] EncryptError),
    #[error(transparent)]
    Compress(#[from] CompressError),
    #[error(transparent)]
    Notification(#[from] NotificationError),
    #[error(transparent)]
    Retention(#[from] RetentionError),
    #[error("{context}: {source}")]
    WithContext { context: String, source: Error },
    #[error("{}", display_multiple(.0))]
    Multiple(Vec<Error>),
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl From<ErrorKind> for Error {
    fn from(e: ErrorKind) -> Self {
        Self(Box::new(e))
    }
}

impl Error {
    pub fn context(self, msg: impl Into<String>) -> Self {
        Self(Box::new(ErrorKind::WithContext {
            context: msg.into(),
            source: self,
        }))
    }

    pub fn multiple(errors: Vec<Error>) -> Self {
        Self(Box::new(ErrorKind::Multiple(errors)))
    }

    pub fn kind(&self) -> &ErrorKind {
        &self.0
    }
}

fn display_multiple(errors: &[Error]) -> String {
    let mut s = format!("{} errors occurred:", errors.len());
    for (i, e) in errors.iter().enumerate() {
        s.push_str(&format!("\n  [{}] {}", i + 1, e));
    }
    s
}

// ─── Context trait ────────────────────────────────────────────────────────

pub trait Context<T> {
    fn context(self, msg: impl Into<String>) -> std::result::Result<T, Error>;
}

impl<T, E: Into<Error>> Context<T> for std::result::Result<T, E> {
    fn context(self, msg: impl Into<String>) -> std::result::Result<T, Error> {
        self.map_err(|e| e.into().context(msg))
    }
}

// ─── Module Errors ────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Config parse error: {0}")]
    Parse(#[from] serde_saphyr::Error),
    #[error("Config validation failed:\n{}", .0.join("\n"))]
    Validation(Vec<String>),
}

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("SQLite backup failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    WalkDir(#[from] walkdir::Error),
    #[error(transparent)]
    StripPrefix(#[from] std::path::StripPrefixError),
    #[error("Base64 decode failed: {0}")]
    Base64(#[from] base64::DecodeError),
}

#[derive(Debug, thiserror::Error)]
pub enum EncryptError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("Invalid recipients: {0}")]
    InvalidRecipients(String),
}

#[derive(Debug, thiserror::Error)]
pub enum CompressError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("LZMA error: {0}")]
    Lzma(#[from] liblzma::stream::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum NotificationError {
    #[error("SMTP: {0}")]
    Smtp(#[from] SmtpError),
    #[error("Command: {0}")]
    Command(#[from] CommandError),
}

#[derive(Debug, thiserror::Error)]
pub enum SmtpError {
    #[error("Transport error")]
    Transport(#[from] lettre::transport::smtp::Error),
    #[error("Invalid address '{address}'")]
    Address {
        address: String,
        #[source]
        source: lettre::address::AddressError,
    },
    #[error("Failed to build email")]
    Build(#[from] lettre::error::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("Failed to spawn {command}")]
    Spawn {
        command: String,
        #[source]
        source: io::Error,
    },
    #[error("Command {command} exited with {status}")]
    NonZeroExit {
        command: String,
        status: String,
        stdout: String,
        stderr: String,
    },
    #[error("Command {command} timed out after {timeout:?}")]
    Timeout { command: String, timeout: Duration },
    #[error("Failed to wait on {command}")]
    Wait {
        command: String,
        #[source]
        source: io::Error,
    },
    #[error("Failed to serialize event")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
#[error("Failed to delete {path}: {source}")]
pub struct RetentionError {
    pub path: PathBuf,
    pub source: io::Error,
}

// ─── Convenience conversions ──────────────────────────────────────────────

impl From<ArchiveError> for Error {
    fn from(e: ArchiveError) -> Self {
        Self(Box::new(ErrorKind::Archive(e)))
    }
}

impl From<EncryptError> for Error {
    fn from(e: EncryptError) -> Self {
        Self(Box::new(ErrorKind::Encrypt(e)))
    }
}

impl From<CompressError> for Error {
    fn from(e: CompressError) -> Self {
        Self(Box::new(ErrorKind::Compress(e)))
    }
}

impl From<NotificationError> for Error {
    fn from(e: NotificationError) -> Self {
        Self(Box::new(ErrorKind::Notification(e)))
    }
}

impl From<SmtpError> for Error {
    fn from(e: SmtpError) -> Self {
        NotificationError::Smtp(e).into()
    }
}

impl From<CommandError> for Error {
    fn from(e: CommandError) -> Self {
        NotificationError::Command(e).into()
    }
}

impl From<RetentionError> for Error {
    fn from(e: RetentionError) -> Self {
        Self(Box::new(ErrorKind::Retention(e)))
    }
}

impl From<ConfigError> for Error {
    fn from(e: ConfigError) -> Self {
        Self(Box::new(ErrorKind::Config(e)))
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self(Box::new(ErrorKind::Io(e)))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_wraps_with_message() {
        let err = Error::from(io::Error::new(io::ErrorKind::NotFound, "file missing"));
        let wrapped = err.context("loading config");
        assert!(wrapped.to_string().contains("loading config"));
        assert!(wrapped.to_string().contains("file missing"));
    }

    #[test]
    fn multiple_displays_numbered_list() {
        let errors = vec![
            Error::from(io::Error::new(io::ErrorKind::Other, "err1")),
            Error::from(io::Error::new(io::ErrorKind::Other, "err2")),
        ];
        let multi = Error::multiple(errors);
        let display = multi.to_string();
        assert!(display.contains("2 errors occurred:"));
        assert!(display.contains("[1]"));
        assert!(display.contains("[2]"));
        assert!(display.contains("err1"));
        assert!(display.contains("err2"));
    }

    #[test]
    fn context_trait_on_result() {
        let result: std::result::Result<(), io::Error> =
            Err(io::Error::new(io::ErrorKind::Other, "inner"));
        let wrapped = result.context("outer");
        let err = wrapped.unwrap_err();
        assert!(err.to_string().contains("outer"));
        assert!(err.to_string().contains("inner"));
    }

    #[test]
    fn from_io_error() {
        let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        let err: Error = io_err.into();
        assert!(err.to_string().contains("denied"));
    }

    #[test]
    fn from_smtp_error() {
        let smtp_err = SmtpError::Build(lettre::error::Error::MissingFrom);
        let err: Error = smtp_err.into();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn from_retention_error() {
        let ret_err = RetentionError {
            path: PathBuf::from("/tmp/backup.age"),
            source: io::Error::new(io::ErrorKind::NotFound, "gone"),
        };
        let err: Error = ret_err.into();
        assert!(err.to_string().contains("/tmp/backup.age"));
    }
}
