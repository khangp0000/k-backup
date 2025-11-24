use crate::backup::arcvec::ArcVec;
use crate::backup::redacted::RedactedString;
use crate::backup::function_path;
use crate::backup::notifications::Notification;
use crate::backup::result_error::error::Error;
use crate::backup::result_error::result::Result;
use crate::backup::result_error::{AddFunctionName, AddMsg};
use bon::Builder;
use getset::Getters;
use function_name::named;
use itertools::Itertools;
use lettre::message::header::ContentType;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::fmt::Display;
use std::ops::Deref;
use validator::Validate;

/// Configuration for SMTP email notifications
///
/// Supports various SMTP modes including SSL, StartTLS, and unsecured connections.
/// Credentials are stored securely using `RedactedString` to prevent exposure
/// in logs and debug output.
#[derive(Clone, Debug, Serialize, Deserialize, Validate, Builder, Getters)]
#[serde(deny_unknown_fields)]
#[serde_as]
#[getset(get = "pub")]
pub struct SmtpNotificationConfig {
    #[builder(into)]
    host: String,
    #[builder(into)]
    smtp_mode: SmtpMode,
    #[builder(into)]
    from: Mailbox,
    #[validate(length(min = 1))]
    #[builder(into)]
    to: ArcVec<Mailbox>,
    #[builder(into)]
    username: String,
    #[builder(into)]
    password: RedactedString,
}

/// SMTP connection security modes
///
/// - `Unsecured`: Plain text connection (not recommended for production)
/// - `Ssl`: SSL/TLS encrypted connection from start
/// - `StartTls`: Start with plain text, then upgrade to TLS
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SmtpMode {
    Unsecured,
    Ssl,
    StartTls,
}

impl Notification for SmtpNotificationConfig {
    #[named]
    fn send<D1: Display, D2: Display>(&self, topic: D1, msg: D2) -> Result<()> {
        tracing::info!(
            "Started smtp email notification from {:?} to {:?}",
            self.from,
            self.to.deref()
        );
        let email = self
            .to
            .iter()
            .fold(Message::builder(), |email, send_to| {
                email.to(send_to.clone())
            })
            .from(self.from.clone())
            .subject(format!("{}", topic))
            .header(ContentType::TEXT_PLAIN)
            .body(format!("{}", msg))
            .map_err(Error::from)
            .add_msg(format!(
                "Fail to build notification email from {:?} to {:?}",
                self.from,
                self.to.deref()
            ))
            .add_fn_name(function_path!())?;

        let creds = Credentials::new(self.username.clone(), self.password.inner().to_string());

        // Open a remote connection to gmail
        let mailer = match self.smtp_mode {
            SmtpMode::Unsecured => Ok(SmtpTransport::builder_dangerous(self.host.as_str())),
            SmtpMode::Ssl => SmtpTransport::relay(self.host.as_str()),
            SmtpMode::StartTls => SmtpTransport::starttls_relay(self.host.as_str()),
        }
        .map_err(Error::from)
        .add_msg(format!(
            "Failed to build smtp client for host: {:?} with mode {:?}",
            self.host, self.smtp_mode
        ))
        .add_fn_name(function_path!())?
        .credentials(creds)
        .build();

        tracing::info!("Sending email...");
        // Send the email
        let response = mailer
            .send(&email)
            .map_err(Error::from)
            .add_fn_name(function_path!())?;
        if response.is_positive() {
            Ok(())
        } else {
            let error_vec = response
                .message()
                .map(|m| Error::smtp_send_error(m.to_owned()))
                .collect_vec();
            Err(Error::lots_of_error(error_vec))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::redacted::RedactedString;
    
    #[test]
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    fn test_smtp_notification_send() {
        use maik;
        use std::env;

        // Skip if running in CI or without network
        if env::var("CI").is_ok() {
            return;
        }

        let server = maik::MockServer::builder().no_verify_credentials().build();

        let config = SmtpNotificationConfig::builder()
            .host(format!("{}:{}", server.host(), server.port()))
            .smtp_mode(SmtpMode::Unsecured)
            .from("test@example.com".parse::<Mailbox>().unwrap())
            .to(vec!["recipient@example.com".parse::<Mailbox>().unwrap()])
            .username("testuser")
            .password(RedactedString::builder().inner("testpass").build())
            .build();

        server.start();
        std::thread::sleep(std::time::Duration::from_millis(100));

        let result = config.send("Test Subject", "Test message body");

        std::thread::sleep(std::time::Duration::from_millis(200));

        if result.is_ok() {
            let assertion = maik::MailAssertion::new()
                .recipients_are(["recipient@example.com"])
                .body_is("Test message body");
            assert!(server.assert(assertion));
        }
    }

    #[test]
    fn test_smtp_notification_validation() {
        let valid_config = SmtpNotificationConfig::builder()
            .host("smtp.example.com")
            .smtp_mode(SmtpMode::Ssl)
            .from("test@example.com".parse::<Mailbox>().unwrap())
            .to(vec!["recipient@example.com".parse::<Mailbox>().unwrap()])
            .username("testuser")
            .password(RedactedString::builder().inner("testpass").build())
            .build();

        assert!(valid_config.validate().is_ok());

        let invalid_config = SmtpNotificationConfig::builder()
            .host("smtp.example.com")
            .smtp_mode(SmtpMode::Ssl)
            .from("test@example.com".parse::<Mailbox>().unwrap())
            .to(vec![])
            .username("testuser")
            .password(RedactedString::builder().inner("testpass").build())
            .build();

        assert!(invalid_config.validate().is_err());
    }

    #[test]
    fn test_smtp_mode_serialization() {
        let modes = vec![
            (SmtpMode::Unsecured, "\"Unsecured\""),
            (SmtpMode::Ssl, "\"Ssl\""),
            (SmtpMode::StartTls, "\"StartTls\""),
        ];

        for (mode, expected) in modes {
            let serialized = serde_json::to_string(&mode).unwrap();
            assert_eq!(serialized, expected);
            let deserialized: SmtpMode = serde_json::from_str(&serialized).unwrap();
            matches!(deserialized, _mode);
        }
    }
}
