use crate::backup::encrypt::{Encryptor, EncryptorBuilder};
use crate::backup::redacted::RedactedString;
use crate::backup::result_error::result::Result;
use derive_more::From;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::result;
use validator::{Validate, ValidationErrors};

/// Configuration for Age encryption
///
/// Age is a modern, secure file encryption tool. Currently only supports
/// passphrase-based encryption (key files not yet implemented).
///
/// The passphrase is stored securely using `RedactedString` which prevents
/// exposure in debug output and logs.
#[derive(From, Clone, Deserialize, Serialize, Debug, PartialEq, Eq)]
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
        passphrase: RedactedString,
    },
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
                    age::Encryptor::with_user_passphrase(passphrase.inner().as_str().into())
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
    /// Validates that Age encryption passphrases meet minimum length requirements
    /// for basic security (8 characters minimum).
    fn validate(&self) -> result::Result<(), ValidationErrors> {
        use validator::{ValidateLength, ValidationError};
        
        match self {
            AgeEncryptorConfig::Passphrase { passphrase } => {
                let mut errors = ValidationErrors::new();
                
                if !passphrase.inner().validate_length(Some(8), None, None) {
                    let mut error = ValidationError::new("length");
                    error.message = Some("Age encryption passphrase must be at least 8 characters long for security".into());
                    errors.add("passphrase", error);
                }
                
                if errors.is_empty() {
                    Ok(())
                } else {
                    Err(errors)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;



    #[test]
    fn test_age_encryptor_config_validation() {
        // Valid configuration
        let valid_config = AgeEncryptorConfig::Passphrase {
            passphrase: RedactedString::builder()
                .inner("valid_passphrase_123")
                .build(),
        };
        assert!(valid_config.validate().is_ok());

        // Invalid configuration (short passphrase - less than 8 characters)
        let invalid_config = AgeEncryptorConfig::Passphrase {
            passphrase: RedactedString::builder().inner("1234567").build(),
        };
        assert!(invalid_config.validate().is_err());
    }

    #[test]
    fn test_build_encryptor() {
        let config = AgeEncryptorConfig::Passphrase {
            passphrase: RedactedString::builder()
                .inner("test_passphrase_123")
                .build(),
        };

        let writer = Cursor::new(Vec::new());
        let encryptor = config.build_encryptor(writer).unwrap();

        match encryptor {
            Encryptor::AgeEncryptor(_) => (),
            _ => panic!("Expected AgeEncryptor"),
        }
    }
}
