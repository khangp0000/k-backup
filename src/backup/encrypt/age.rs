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

static REDACTED_PASSPHRASE: &str = "###REDACTED_PASSPHRASE###";

#[derive(From, Clone, Deserialize, Serialize, Debug)]
#[serde(tag = "secret_type")]
#[serde(rename_all = "snake_case")]
pub enum AgeEncryptorConfig {
    Passphrase { passphrase: Secret<RedactedString> },
}

#[derive(Validate, Clone, From)]
pub struct RedactedString {
    #[validate(length(min = 8))]
    inner: String,
}

impl Debug for RedactedString {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.serialize_str(REDACTED_PASSPHRASE)
    }
}

impl Serialize for RedactedString {
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
    fn build_encryptor(&self, writer: W) -> Result<Encryptor<W>> {
        match self {
            AgeEncryptorConfig::Passphrase { passphrase } => {
                Ok(age::Encryptor::with_user_passphrase(
                    passphrase.expose_secret().inner.clone().into(),
                )
                .wrap_output(writer)
                .map_err(|e| match e {
                    EncryptError::Io(e) => e,
                    _ => panic!("Unexpected or supported error occurred: {e}"),
                })?
                .into())
            }
        }
    }
}

impl Validate for AgeEncryptorConfig {
    fn validate(&self) -> result::Result<(), ValidationErrors> {
        match self {
            AgeEncryptorConfig::Passphrase { passphrase } => passphrase.expose_secret().validate(),
        }
    }
}
