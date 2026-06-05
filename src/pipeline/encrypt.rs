//! Age encryption Write wrapper.

use crate::config::{AgeConfig, EncryptorConfig};
use crate::error::EncryptError;
use crate::pipeline::FinishableWrite;
use age::secrecy::SecretString;
use std::io::Write;

/// Wrapper that can properly finish() the age stream writer.
pub enum AgeWriter {
    Passthrough(Box<dyn FinishableWrite>),
    Age(age::stream::StreamWriter<Box<dyn FinishableWrite>>),
}

impl Write for AgeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Passthrough(w) => w.write(buf),
            Self::Age(w) => w.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Passthrough(w) => w.flush(),
            Self::Age(w) => w.flush(),
        }
    }
}

impl FinishableWrite for AgeWriter {
    fn finish(self: Box<Self>) -> std::io::Result<()> {
        match *self {
            Self::Passthrough(w) => w.finish(),
            Self::Age(w) => {
                let inner = w.finish()?;
                inner.finish()
            }
        }
    }
}

/// Wraps a writer with encryption. Returns Box<dyn FinishableWrite>.
pub fn wrap_writer(
    config: &EncryptorConfig,
    writer: Box<dyn FinishableWrite>,
) -> std::result::Result<Box<dyn FinishableWrite>, EncryptError> {
    match config {
        EncryptorConfig::None => Ok(Box::new(AgeWriter::Passthrough(writer))),
        EncryptorConfig::Age(age_config) => match age_config {
            AgeConfig::Passphrase { passphrase } => {
                let encryptor = age::Encryptor::with_user_passphrase(SecretString::new(
                    passphrase.inner().into(),
                ));
                let age_writer = encryptor
                    .wrap_output(writer)
                    .map_err(|e| EncryptError::Io(std::io::Error::other(e)))?;
                Ok(Box::new(AgeWriter::Age(age_writer)))
            }
            AgeConfig::RecipientsFiles { recipients_files } => {
                let file_strings: Vec<String> = recipients_files
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();

                let mut stdin_guard = age::cli_common::StdinGuard::new(false);
                let recipients = age::cli_common::read_recipients(
                    vec![],
                    file_strings,
                    vec![],
                    None,
                    &mut stdin_guard,
                )
                .map_err(|e| EncryptError::InvalidRecipients(e.to_string()))?;

                if recipients.is_empty() {
                    return Err(EncryptError::InvalidRecipients(
                        "No recipients found".into(),
                    ));
                }

                let encryptor =
                    age::Encryptor::with_recipients(recipients.iter().map(|r| r.as_ref() as _))
                        .map_err(|e| EncryptError::InvalidRecipients(e.to_string()))?;

                let age_writer = encryptor
                    .wrap_output(writer)
                    .map_err(|e| EncryptError::Io(std::io::Error::other(e)))?;
                Ok(Box::new(AgeWriter::Age(age_writer)))
            }
        },
    }
}
