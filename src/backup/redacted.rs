use crate::backup::encrypt::age::RedactedString;
use serde::de::Visitor;
use std::fmt::{Debug, Formatter};
use std::result;

/// Placeholder text shown instead of actual passphrase in logs/debug output
pub static REDACTED_PASSPHRASE: &str = "###REDACTED_PASSPHRASE###";

impl Debug for RedactedString {
    /// Always shows redacted placeholder instead of actual value
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", REDACTED_PASSPHRASE)
    }
}

pub struct RedactedStringVisitor;

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
