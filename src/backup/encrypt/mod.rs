pub mod age;

use crate::backup::encrypt::age::AgeEncryptorConfig;
use crate::backup::file_ext::FileExtProvider;
use crate::backup::finish::Finish;
use crate::backup::result_error::result::Result;
use crate::backup::result_error::WithDebugObjectAndFnName;
use ::age::stream::StreamWriter;
use derive_more::From;
use io_enum::Write;
use serde::{Deserialize, Serialize};
use std::io::{Error, Write};
use std::result;
use std::sync::{Arc, OnceLock};
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
            EncryptorConfig::None => Ok(writer.into()),
            EncryptorConfig::Age(age) => age.build_encryptor(writer),
        }
        .with_debug_object_and_fn_name(self.clone(), "build_encryptor")
    }
}

static AGE_FILE_EXT: OnceLock<Arc<str>> = OnceLock::new();
impl FileExtProvider for EncryptorConfig {
    fn file_ext(&self) -> Option<Arc<str>> {
        match self {
            EncryptorConfig::None => None,
            EncryptorConfig::Age(_) => Some(AGE_FILE_EXT.get_or_init(|| "age".into()).clone()),
        }
    }
}
