pub mod age;

use crate::backup::encrypt::age::AgeEncryptorConfig;
use crate::backup::file_ext::FileExtProvider;
use crate::backup::finish::Finish;
use crate::backup::result_error::result::Result;
use crate::backup::result_error::AddDebugObjectAndFnName;
use ::age::stream::StreamWriter;
use derive_more::From;
use io_enum::Write;
use serde::{Deserialize, Serialize};
use std::io::{Error, Write};
use std::result;

use validator::{Validate, ValidationErrors};

#[derive(Write, From)]
pub enum Encryptor<W: Write> {
    None(W),
    AgeEncryptor(StreamWriter<W>),
}

#[derive(Clone, Default, From, Serialize, Deserialize, Debug)]
#[serde(tag = "encryptor_type")]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum EncryptorConfig {
    #[default]
    None,
    Age(AgeEncryptorConfig),
}

impl Validate for EncryptorConfig {
    fn validate(&self) -> result::Result<(), ValidationErrors> {
        match self {
            EncryptorConfig::None => Ok(()),
            EncryptorConfig::Age(inner) => inner.validate(),
        }
    }
}

pub trait EncryptorBuilder<W: Write> {
    fn build_encryptor(&self, writer: W) -> Result<Encryptor<W>>;
}

impl<W: Write> Finish<W> for Encryptor<W> {
    fn finish(self) -> result::Result<W, Error> {
        match self {
            Encryptor::None(w) => Ok(w),
            Encryptor::AgeEncryptor(w) => w.finish(),
        }
    }
}

impl<W: Write> EncryptorBuilder<W> for EncryptorConfig {
    fn build_encryptor(&self, writer: W) -> Result<Encryptor<W>> {
        match self {
            EncryptorConfig::None => {
                tracing::info!("Using no encryption");
                Ok(writer.into())
            }
            EncryptorConfig::Age(age) => {
                tracing::info!("Initializing Age encryption with passphrase");
                age.build_encryptor(writer)
            }
        }
        .add_debug_object_and_fn_name(self.clone(), "build_encryptor")
    }
}

impl FileExtProvider for EncryptorConfig {
    fn file_ext(&self) -> Option<impl AsRef<str>> {
        match self {
            EncryptorConfig::None => None,
            EncryptorConfig::Age(_) => Some("age"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::encrypt::age::{AgeEncryptorConfig, RedactedString};
    use std::io::Cursor;

    #[test]
    fn test_encryptor_config_none() {
        let config = EncryptorConfig::None;
        assert!(config.validate().is_ok());
        assert!(config.file_ext().is_none());
    }

    #[test]
    fn test_encryptor_config_age() {
        let config = EncryptorConfig::Age(AgeEncryptorConfig::Passphrase {
            passphrase: RedactedString::from("test_passphrase_123"),
        });
        assert!(config.validate().is_ok());
        assert!(config.file_ext().is_some());
        assert_eq!(config.file_ext().unwrap().as_ref(), "age");
    }

    #[test]
    fn test_encryptor_builder_none() {
        let config = EncryptorConfig::None;
        let writer = Cursor::new(Vec::new());
        let encryptor = config.build_encryptor(writer).unwrap();

        match encryptor {
            Encryptor::None(_) => (),
            _ => panic!("Expected None encryptor"),
        }
    }

    #[test]
    fn test_encryptor_finish_none() {
        let writer = Cursor::new(Vec::new());
        let encryptor = Encryptor::None(writer);
        let result = encryptor.finish();
        assert!(result.is_ok());
    }

    #[test]
    fn test_encryptor_config_serialization() {
        let config = EncryptorConfig::None;
        let serialized = serde_json::to_string(&config).unwrap();
        assert_eq!(serialized, "{\"encryptor_type\":\"none\"}");

        let deserialized: EncryptorConfig = serde_json::from_str(&serialized).unwrap();
        matches!(deserialized, EncryptorConfig::None);
    }

    #[test]
    fn test_encryptor_config_default() {
        let config = EncryptorConfig::default();
        matches!(config, EncryptorConfig::None);
    }
}
