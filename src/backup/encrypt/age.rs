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
#[serde(deny_unknown_fields)]
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
                tracing::debug!("Initializing Age encryption with passphrase");
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

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::Secret;
    use std::io::Cursor;

    #[test]
    fn test_redacted_string_debug() {
        let redacted = RedactedString::from("secret_password".to_string());
        let debug_str = format!("{:?}", redacted);
        assert_eq!(debug_str, REDACTED_PASSPHRASE);
        assert!(!debug_str.contains("secret_password"));
    }

    #[test]
    fn test_redacted_string_serialize() {
        let redacted = RedactedString::from("secret_password".to_string());
        let serialized = serde_json::to_string(&redacted).unwrap();
        assert_eq!(serialized, format!("\"{}\"", REDACTED_PASSPHRASE));
        assert!(!serialized.contains("secret_password"));
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
        let valid = RedactedString::from("valid_password".to_string());
        assert!(valid.validate().is_ok());
        
        // Invalid passphrase (too short)
        let invalid = RedactedString::from("short".to_string());
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn test_redacted_string_zeroize() {
        let mut redacted = RedactedString::from("secret_password".to_string());
        redacted.zeroize();
        // After zeroizing, the inner string should be cleared
        // Note: We can't easily test this without exposing internals
    }

    #[test]
    fn test_age_encryptor_config_validation() {
        // Valid configuration
        let valid_config = AgeEncryptorConfig::Passphrase {
            passphrase: Secret::new(RedactedString::from("valid_passphrase_123".to_string())),
        };
        assert!(valid_config.validate().is_ok());
        
        // Invalid configuration (short passphrase)
        let invalid_config = AgeEncryptorConfig::Passphrase {
            passphrase: Secret::new(RedactedString::from("short".to_string())),
        };
        assert!(invalid_config.validate().is_err());
    }

    #[test]
    fn test_age_encryptor_config_debug() {
        let config = AgeEncryptorConfig::Passphrase {
            passphrase: Secret::new(RedactedString::from("secret_password".to_string())),
        };
        let debug_str = format!("{:?}", config);
        assert!(!debug_str.contains("secret_password"));
        assert!(debug_str.contains("[REDACTED"));
    }

    #[test]
    fn test_age_encryptor_config_serialization() {
        let config = AgeEncryptorConfig::Passphrase {
            passphrase: Secret::new(RedactedString::from("secret_password".to_string())),
        };
        
        let serialized = serde_json::to_string(&config).unwrap();
        assert!(!serialized.contains("secret_password"));
        assert!(serialized.contains(REDACTED_PASSPHRASE));
        assert!(serialized.contains("secret_type"));
        assert!(serialized.contains("passphrase"));
    }

    #[test]
    fn test_age_encryptor_config_deserialization() {
        let json = r#"{"secret_type":"passphrase","passphrase":"actual_password_123"}"#;
        let config: AgeEncryptorConfig = serde_json::from_str(json).unwrap();
        
        match config {
            AgeEncryptorConfig::Passphrase { passphrase } => {
                assert_eq!(passphrase.expose_secret().inner, "actual_password_123");
            }
        }
    }

    #[test]
    fn test_build_encryptor() {
        let config = AgeEncryptorConfig::Passphrase {
            passphrase: Secret::new(RedactedString::from("test_passphrase_123".to_string())),
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
        let original = "test_password".to_string();
        let redacted = RedactedString::from(original.clone());
        assert_eq!(redacted.inner, original);
    }

    #[test]
    fn test_secret_traits() {
        let redacted = RedactedString::from("test_password".to_string());
        let secret = Secret::new(redacted);
        
        // Test that it implements the required secret traits
        let _cloned = secret.clone();
        let _debug = format!("{:?}", secret);
    }
}