use crate::backup::encrypt::{Encryptor, EncryptorBuilder};
use crate::backup::redacted::RedactedString;
use crate::backup::result_error::result::Result;
use age::cli_common::{read_recipients, StdinGuard};
use derive_more::From;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::result;
use validator::{Validate, ValidationErrors};

/// Configuration for Age encryption
///
/// Age is a modern, secure file encryption tool. Supports passphrase-based
/// encryption and recipient file-based encryption (x25519, SSH keys, plugins).
#[derive(From, Clone, Deserialize, Serialize, Debug, PartialEq, Eq)]
#[serde(tag = "secret_type")]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum AgeEncryptorConfig {
    /// Passphrase-based encryption
    Passphrase {
        /// The encryption passphrase (stored securely, redacted in logs)
        passphrase: RedactedString,
    },
    /// Recipient file-based encryption
    ///
    /// Reads recipients from file(s). Each file contains one recipient per line
    /// (age x25519, SSH public keys, or plugin recipients). Comments (#) and
    /// empty lines are ignored.
    RecipientsFiles {
        /// List of paths to recipient files
        recipients_files: Vec<String>,
    },
}

impl<W: Write> EncryptorBuilder<W> for AgeEncryptorConfig {
    fn build_encryptor(&self, writer: W) -> Result<Encryptor<W>> {
        match self {
            AgeEncryptorConfig::Passphrase { passphrase } => {
                tracing::debug!("Initializing Age encryption with passphrase");
                Ok(
                    age::Encryptor::with_user_passphrase(passphrase.inner().as_str().into())
                        .wrap_output(writer)?
                        .into(),
                )
            }
            AgeEncryptorConfig::RecipientsFiles { recipients_files } => {
                tracing::debug!("Initializing Age encryption with recipients file(s)");
                let mut stdin_guard = StdinGuard::new(true);
                let recipients = read_recipients(
                    vec![],
                    recipients_files.clone(),
                    vec![],
                    None,
                    &mut stdin_guard,
                )
                .map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string())
                })?;

                let encryptor =
                    age::Encryptor::with_recipients(recipients.iter().map(|r| r.as_ref() as _))
                        .map_err(|e| {
                            std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string())
                        })?;

                Ok(encryptor.wrap_output(writer)?.into())
            }
        }
    }
}

impl Validate for AgeEncryptorConfig {
    fn validate(&self) -> result::Result<(), ValidationErrors> {
        use validator::{ValidateLength, ValidationError};

        match self {
            AgeEncryptorConfig::Passphrase { passphrase } => {
                let mut errors = ValidationErrors::new();

                if !passphrase.inner().validate_length(Some(8), None, None) {
                    let mut error = ValidationError::new("length");
                    error.message = Some(
                        "Age encryption passphrase must be at least 8 characters long for security"
                            .into(),
                    );
                    errors.add("passphrase", error);
                }

                if errors.is_empty() {
                    Ok(())
                } else {
                    Err(errors)
                }
            }
            AgeEncryptorConfig::RecipientsFiles { recipients_files } => {
                let mut errors = ValidationErrors::new();

                if recipients_files.is_empty() {
                    let mut error = ValidationError::new("length");
                    error.message = Some("At least one recipients file must be specified".into());
                    errors.add("recipients_files", error);
                }

                for path in recipients_files {
                    if !std::path::Path::new(path).exists() {
                        let mut error = ValidationError::new("file_exists");
                        error.message = Some(format!("Recipients file not found: {}", path).into());
                        errors.add("recipients_files", error);
                    }
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
        // Valid passphrase configuration
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
    fn test_recipients_file_validation_empty_list() {
        let config = AgeEncryptorConfig::RecipientsFiles {
            recipients_files: vec![],
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_recipients_file_validation_missing_file() {
        let config = AgeEncryptorConfig::RecipientsFiles {
            recipients_files: vec!["/nonexistent/path/recipients.txt".into()],
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_recipients_file_validation_existing_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config = AgeEncryptorConfig::RecipientsFiles {
            recipients_files: vec![tmp.path().to_string_lossy().into_owned()],
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_build_encryptor_passphrase() {
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

    #[test]
    fn test_build_encryptor_recipients_file() {
        // Generate a key and write the public key to a temp file
        let key = age::x25519::Identity::generate();
        let pubkey = key.to_public().to_string();

        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), format!("{}\n", pubkey)).unwrap();

        let config = AgeEncryptorConfig::RecipientsFiles {
            recipients_files: vec![tmp.path().to_string_lossy().into_owned()],
        };

        let writer = Cursor::new(Vec::new());
        let encryptor = config.build_encryptor(writer).unwrap();

        match encryptor {
            Encryptor::AgeEncryptor(_) => (),
            _ => panic!("Expected AgeEncryptor"),
        }
    }

    #[test]
    fn test_recipients_file_deserialization() {
        let yaml = r#"
secret_type: recipients_files
recipients_files:
  - /path/to/recipients1.txt
  - /path/to/recipients2.txt
"#;
        let config: AgeEncryptorConfig = serde_yml::from_str(yaml).unwrap();
        match config {
            AgeEncryptorConfig::RecipientsFiles { recipients_files } => {
                assert_eq!(recipients_files.len(), 2);
                assert_eq!(recipients_files[0], "/path/to/recipients1.txt");
                assert_eq!(recipients_files[1], "/path/to/recipients2.txt");
            }
            _ => panic!("Expected RecipientsFile variant"),
        }
    }
}
