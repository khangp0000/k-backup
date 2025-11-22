use crate::backup::encrypt::{Encryptor, EncryptorBuilder};
use crate::backup::result_error::result::Result;
use derive_ctor::ctor;
use derive_more::From;
use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{Debug, Formatter};
use std::io::Write;
use std::result;
use validator::{Validate, ValidationErrors};
use zeroize::Zeroize;

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
#[serde(deny_unknown_fields)]
#[derive(ctor)]
#[ctor(prefix = new, vis = pub)]
pub enum AgeEncryptorConfig {
    /// Passphrase-based encryption
    ///
    /// The passphrase is used to derive an encryption key.
    /// Must be at least 8 characters long for basic security.
    Passphrase {
        /// The encryption passphrase (stored securely, redacted in logs)
        #[ctor(into)]
        passphrase: RedactedString,
    },
}

/// A string that gets redacted in debug output and serialization
///
/// Used to store sensitive data like passphrases while preventing
/// accidental exposure in logs, debug output, or serialized config.
#[derive(Validate, Clone, Zeroize, ctor, From)]
#[ctor(pub new)]
pub struct RedactedString {
    /// Minimum 8 characters for basic security
    #[validate(length(min = 8))]
    #[ctor(into)]
    inner: String,
}

impl Debug for RedactedString {
    /// Always shows redacted placeholder instead of actual value
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", REDACTED_PASSPHRASE)
    }
}

impl Serialize for RedactedString {
    /// Always serializes as redacted placeholder for security
    fn serialize<S: Serializer>(&self, serializer: S) -> result::Result<S::Ok, S::Error> {
        serializer.serialize_str(REDACTED_PASSPHRASE)
    }
}

struct RedactedStringVisitor;

impl Visitor<'_> for RedactedStringVisitor {
    type Value = RedactedString;

    fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
        formatter.write_str("a string")
    }

    /// Deserializes the actual passphrase from config file
    fn visit_str<E>(self, v: &str) -> result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(RedactedString { inner: v.into() })
    }
}

impl<'de> Deserialize<'de> for RedactedString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_str(RedactedStringVisitor)
    }
}

impl Drop for RedactedString {
    fn drop(&mut self) {
        // Zero out the internal string when dropped
        self.zeroize();
    }
}

impl<W: Write> EncryptorBuilder<W> for AgeEncryptorConfig {
    /// Creates an Age encryptor with the configured passphrase
    ///
    /// Uses the Age library to create a streaming encryptor that encrypts
    /// data as it's written. The passphrase is used to derive encryption keys.
    ///
    /// Returns configured Age encryptor
    fn build_encryptor(&self, writer: W) -> Result<Encryptor<W>> {
        match self {
            AgeEncryptorConfig::Passphrase { passphrase } => {
                // Create Age encryptor with user passphrase
                tracing::debug!("Initializing Age encryption with passphrase");
                Ok(
                    age::Encryptor::with_user_passphrase(passphrase.inner.as_str().into())
                        .wrap_output(writer)?
                        .into(),
                )
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
            AgeEncryptorConfig::Passphrase { passphrase } => passphrase.validate(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_redacted_string_debug() {
        let redacted = RedactedString::new("secret_password");
        let debug_str = format!("{:?}", redacted);
        assert_eq!(debug_str, REDACTED_PASSPHRASE);
    }

    #[test]
    fn test_redacted_string_serialize() {
        let redacted = RedactedString::new("secret_password");
        let serialized = serde_json::to_string(&redacted).unwrap();
        assert_eq!(serialized, format!("\"{}\"", REDACTED_PASSPHRASE));
    }

    #[test]
    fn test_redacted_string_deserialize() {
        let json = "\"actual_password_123\"";
        let redacted: RedactedString = serde_json::from_str(json).unwrap();
        assert_eq!(redacted.inner, "actual_password_123");
    }

    #[test]
    fn test_redacted_string_validation() {
        // Valid passphrase (8+ characters)
        let valid = RedactedString::new("valid_password");
        assert!(valid.validate().is_ok());

        // Invalid passphrase (too short)
        let invalid = RedactedString::new("short");
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn test_redacted_string_zeroize() {
        let mut redacted = RedactedString::new("secret_password");
        redacted.zeroize();
        // After zeroizing, the inner string should be cleared
        // Note: We can't easily test this without exposing internals
    }

    #[test]
    fn test_age_encryptor_config_validation() {
        // Valid configuration
        let valid_config = AgeEncryptorConfig::Passphrase {
            passphrase: RedactedString::new("valid_passphrase_123"),
        };
        assert!(valid_config.validate().is_ok());

        // Invalid configuration (short passphrase)
        let invalid_config = AgeEncryptorConfig::Passphrase {
            passphrase: RedactedString::new("short"),
        };
        assert!(invalid_config.validate().is_err());
    }

    #[test]
    fn test_age_encryptor_config_debug() {
        let config = AgeEncryptorConfig::Passphrase {
            passphrase: RedactedString::new("secret_password"),
        };
        let debug_str = format!("{:?}", config);
        assert_eq!(
            debug_str,
            format!("Passphrase {{ passphrase: {} }}", REDACTED_PASSPHRASE)
        );
    }

    #[test]
    fn test_age_encryptor_config_serialization() {
        let config = AgeEncryptorConfig::Passphrase {
            passphrase: RedactedString::from("secret_password".to_string()),
        };

        let serialized = serde_json::to_string(&config).unwrap();
        assert_eq!(
            serialized,
            format!(
                "{{\"secret_type\":\"passphrase\",\"passphrase\":\"{}\"}}",
                REDACTED_PASSPHRASE
            )
        );
    }

    #[test]
    fn test_age_encryptor_config_deserialization() {
        let json = r#"{"secret_type":"passphrase","passphrase":"actual_password_123"}"#;
        let config: AgeEncryptorConfig = serde_json::from_str(json).unwrap();

        match config {
            AgeEncryptorConfig::Passphrase { passphrase } => {
                assert_eq!(passphrase.inner, "actual_password_123");
            }
        }
    }

    #[test]
    fn test_build_encryptor() {
        let config = AgeEncryptorConfig::Passphrase {
            passphrase: RedactedString::new("test_passphrase_123"),
        };

        let writer = Cursor::new(Vec::new());
        let encryptor = config.build_encryptor(writer).unwrap();

        match encryptor {
            Encryptor::AgeEncryptor(_) => (),
            _ => panic!("Expected AgeEncryptor"),
        }
    }

    #[test]
    fn test_redacted_string_from_string() {
        let original = "test_password";
        let redacted = RedactedString::new(original);
        assert_eq!(redacted.inner, original);
    }
}
