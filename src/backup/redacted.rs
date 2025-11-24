//! Secure string handling with redacted display and serialization.
//!
//! Provides `RedactedString` for storing sensitive data like passwords while preventing
//! accidental exposure in logs, debug output, or serialized configuration.

use bon::Builder;
use getset::Getters;
use derive_more::From;
use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{Debug, Formatter};
use std::result;
use validator::Validate;
use zeroize::Zeroize;

/// Placeholder text shown instead of actual passphrase in logs/debug output
pub static REDACTED_PASSPHRASE: &str = "###REDACTED_PASSPHRASE###";

/// A string that gets redacted in debug output and serialization
///
/// Used to store sensitive data like passphrases while preventing
/// accidental exposure in logs, debug output, or serialized config.
/// 
/// Provides secure access through getter methods and automatically
/// zeros memory on drop for additional security.
#[derive(Validate, Clone, Zeroize, From, Builder, PartialEq, Eq, Getters)]
#[getset(get = "pub")]
pub struct RedactedString {
    /// Minimum 8 characters for basic security
    #[validate(length(min = 8))]
    #[builder(into)]
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

impl<'de> Deserialize<'de> for RedactedString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> result::Result<Self, D::Error> {
        deserializer.deserialize_str(RedactedStringVisitor)
    }
}

impl Drop for RedactedString {
    fn drop(&mut self) {
        // Zero out the internal string when dropped
        self.zeroize();
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
        Ok(RedactedString::builder().inner(v).build())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redacted_string_validation() {
        // Valid passphrase (8+ characters)
        let valid = RedactedString::builder().inner("valid_password").build();
        assert!(valid.validate().is_ok());

        // Invalid passphrase (too short)
        let invalid = RedactedString::builder().inner("short").build();
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn test_redacted_string_zeroize() {
        let mut redacted = RedactedString::builder().inner("secret_password").build();
        redacted.zeroize();
        // After zeroizing, the inner string should be cleared
        // Note: We can't easily test this without exposing internals
    }
}
