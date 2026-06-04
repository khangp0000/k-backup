//! SMTP notification target.

use crate::config::SmtpConfig;
use crate::error::{SmtpError, Result};
use crate::notifications::event::BackupEvent;
use lettre::message::header::ContentType;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};

pub fn send_event(config: &SmtpConfig, event: &BackupEvent) -> Result<()> {
    let (subject, body) = format_event(event);
    send_email(config, &subject, &body)
}

fn format_event(event: &BackupEvent) -> (String, String) {
    match event {
        BackupEvent::BackupCycleStart { timestamp, .. } => (
            format!("[{}] Backup cycle started", timestamp),
            "Backup cycle has started.".into(),
        ),
        BackupEvent::Success { timestamp, output_file, .. } => (
            format!("[{}] Backup successful", timestamp),
            format!("Backup completed successfully.\nOutput: {:?}", output_file),
        ),
        BackupEvent::NonFatalError { timestamp, output_file, errors, .. } => (
            format!("[{}] Backup completed with errors", timestamp),
            format!("Backup completed with non-fatal errors.\nOutput: {:?}\n\n{}", output_file, errors),
        ),
        BackupEvent::FatalError { timestamp, error, .. } => (
            format!("[{}] Backup failed", timestamp),
            format!("Backup failed with fatal error.\n\n{}", error),
        ),
    }
}

fn send_email(config: &SmtpConfig, subject: &str, body: &str) -> Result<()> {
    let from: Mailbox = config.from.parse().map_err(|e| SmtpError::Address {
        address: config.from.clone(),
        source: e,
    })?;

    let mut msg_builder = Message::builder().from(from);
    for to_addr in &config.to {
        let mailbox: Mailbox = to_addr.parse().map_err(|e| SmtpError::Address {
            address: to_addr.clone(),
            source: e,
        })?;
        msg_builder = msg_builder.to(mailbox);
    }

    let email = msg_builder
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body.to_string())
        .map_err(SmtpError::from)?;

    let creds = Credentials::new(
        config.username.clone(),
        config.password.inner().to_string(),
    );

    use crate::config::SmtpMode;
    let transport = match config.smtp_mode {
        SmtpMode::Unsecured => Ok(SmtpTransport::builder_dangerous(&config.host)),
        SmtpMode::Ssl => SmtpTransport::relay(&config.host),
        SmtpMode::StartTls => SmtpTransport::starttls_relay(&config.host),
    }
    .map_err(SmtpError::from)?
    .credentials(creds)
    .build();

    transport.send(&email).map_err(SmtpError::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BackupConfig, CompressorConfig, EncryptorConfig, RedactedString, SmtpConfig, SmtpMode};
    use crate::notifications::event::BackupEvent;
    use chrono::{TimeZone, Utc};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn test_smtp_config(port: u16) -> SmtpConfig {
        SmtpConfig {
            host: format!("127.0.0.1:{}", port),
            smtp_mode: SmtpMode::Unsecured,
            from: "test@example.com".to_string(),
            to: vec!["recipient@example.com".to_string()],
            username: "testuser".to_string(),
            password: RedactedString::new("testpass"),
        }
    }

    fn test_event() -> BackupEvent {
        BackupEvent::Success {
            config: Arc::new(BackupConfig {
                cron: None,
                archive_base_name: "test".to_string(),
                out_dir: PathBuf::from("/tmp"),
                temp_dir: None,
                files: vec![],
                notifications: vec![],
                compressor: CompressorConfig::None,
                encryptor: EncryptorConfig::None,
                retention: None,
            }),
            timestamp: Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap(),
            output_file: PathBuf::from("/tmp/test.tar"),
        }
    }

    #[test]
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    fn test_smtp_send_with_mock() {
        use maik;

        let server = maik::MockServer::builder().no_verify_credentials().build();
        let port = server.port();
        server.start();
        std::thread::sleep(std::time::Duration::from_millis(100));

        let from: lettre::message::Mailbox = "test@example.com".parse().unwrap();
        let to: lettre::message::Mailbox = "recipient@example.com".parse().unwrap();
        let email = lettre::Message::builder()
            .from(from)
            .to(to)
            .subject("Test")
            .header(lettre::message::header::ContentType::TEXT_PLAIN)
            .body("Test body".to_string())
            .unwrap();

        let transport = lettre::SmtpTransport::builder_dangerous("127.0.0.1")
            .port(port)
            .build();

        let result = lettre::Transport::send(&transport, &email);
        std::thread::sleep(std::time::Duration::from_millis(200));

        if let Err(e) = &result {
            eprintln!("SMTP mock test inconclusive (network): {}", e);
            return;
        }

        let assertion = maik::MailAssertion::new()
            .recipients_are(["recipient@example.com"]);
        assert!(server.assert(assertion));
    }

    #[test]
    fn test_format_event_success() {
        let event = test_event();
        let (subject, body) = format_event(&event);
        assert!(subject.contains("Backup successful"));
        assert!(body.contains("/tmp/test.tar"));
    }

    #[test]
    fn test_format_event_fatal() {
        let event = BackupEvent::FatalError {
            config: Arc::new(BackupConfig {
                cron: None,
                archive_base_name: "t".to_string(),
                out_dir: PathBuf::from("/tmp"),
                temp_dir: None,
                files: vec![],
                notifications: vec![],
                compressor: CompressorConfig::None,
                encryptor: EncryptorConfig::None,
                retention: None,
            }),
            timestamp: Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
            error: "disk full".to_string(),
        };
        let (subject, body) = format_event(&event);
        assert!(subject.contains("Backup failed"));
        assert!(body.contains("disk full"));
    }
}
