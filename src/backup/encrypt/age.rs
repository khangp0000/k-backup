use crate::backup::encrypt::{Encryptor, EncryptorBuilder};
use crate::backup::result_error::result::Result;
use age::EncryptError;
use derive_more::From;
use secrecy::{CloneableSecret, DebugSecret, ExposeSecret, Secret, SerializableSecret, Zeroize};
use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{Debug, Formatter};
use std::io::Write;
use std::result;
use validator::{Validate, ValidationErrors};

/// Placeholder text shown instead of actual passphrase in logs/debug output
static REDACTED_PASSPHRASE: &str = "###REDACTED_PASSPHRASE###";

/// Configuration for Age encryption
/// 
/// Age is a modern, secure file encryption tool. Currently only supports
/// passphrase-based encryption (key files not yet implemented).
/// 
/// The passphrase is stored securely in memory and redacted from debug output.
#[derive(From, Clone, Deserialize, Serialize, Debug)]
#[serde(tag = "secret_type")]
#[serde(rename_all = "snake_case")]
pub enum AgeEncryptorConfig {
    /// Passphrase-based encryption
    /// 
    /// The passphrase is used to derive an encryption key.
    /// Must be at least 8 characters long for basic security.
    Passphrase { 
        /// The encryption passphrase (stored securely, redacted in logs)
        passphrase: Secret<RedactedString> 
    },
}

/// A string that gets redacted in debug output and serialization
/// 
/// Used to store sensitive data like passphrases while preventing
/// accidental exposure in logs, debug output, or serialized config.
#[derive(Validate, Clone, From)]
pub struct RedactedString {
    /// Minimum 8 characters for basic security
    #[validate(length(min = 8))]
    inner: String,
}

impl Debug for RedactedString {
    /// Always shows redacted placeholder instead of actual value
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.serialize_str(REDACTED_PASSPHRASE)
    }
}

impl Serialize for RedactedString {
    /// Always serializes as redacted placeholder for security
    fn serialize<S: Serializer>(&self, serializer: S) -> result::Result<S::Ok, S::Error> {
        serializer.serialize_str(REDACTED_PASSPHRASE)
    }
}

struct RedactedStringVisitor;

impl<'de> Visitor<'de> for RedactedStringVisitor {
    type Value = RedactedString;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a string")
    }

    /// Deserializes the actual passphrase from config file
    fn visit_str<E>(self, v: &str) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(v.to_string().into())
    }
}

impl<'de> Deserialize<'de> for RedactedString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_str(RedactedStringVisitor)
    }
}

impl Zeroize for RedactedString {
    fn zeroize(&mut self) {
        self.inner.zeroize()
    }
}

impl SerializableSecret for RedactedString {}
impl DebugSecret for RedactedString {}
impl CloneableSecret for RedactedString {}

impl<W: Write> EncryptorBuilder<W> for AgeEncryptorConfig {
    /// Creates an Age encryptor with the configured passphrase
    /// 
    /// Uses the Age library to create a streaming encryptor that encrypts
    /// data as it's written. The passphrase is used to derive encryption keys.
    /// 
    /// FIXME: This panics on unexpected errors instead of handling them gracefully.
    fn build_encryptor(&self, writer: W) -> Result<Encryptor<W>> {
        match self {
            AgeEncryptorConfig::Passphrase { passphrase } => {
                // Create Age encryptor with user passphrase
                Ok(age::Encryptor::with_user_passphrase(
                    passphrase.expose_secret().inner.clone().into(),
                )
                .wrap_output(writer)
                .map_err(|e| match e {
                    EncryptError::Io(e) => e,
                    // FIXME: This panic will crash the backup daemon
                    // Should return a proper error instead
                    _ => panic!("Unexpected or supported error occurred: {e}"),
                })?
                .into())
            }
        }
    }
}

impl Validate for AgeEncryptorConfig {
    /// Validates the encryption configuration
    /// 
    /// Currently only validates that passphrases meet minimum length requirements.
    fn validate(&self) -> result::Result<(), ValidationErrors> {
        match self {
            AgeEncryptorConfig::Passphrase { passphrase } => passphrase.expose_secret().validate(),
        }
    }
}
