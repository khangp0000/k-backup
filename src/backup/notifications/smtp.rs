use crate::backup::arcvec::ArcVec;
use crate::backup::encrypt::age::RedactedString;
use crate::backup::function_path;
use crate::backup::notifications::Notification;
use crate::backup::result_error::error::Error;
use crate::backup::result_error::result::Result;
use crate::backup::result_error::{AddFunctionName, AddMsg};
use derive_ctor::ctor;
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

#[derive(Clone, Debug, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
#[derive(ctor)]
#[ctor(pub new)]
#[serde_as]
pub struct SmtpNotificationConfig {
    #[ctor(into)]
    host: String,
    #[ctor(into)]
    smtp_mode: SmtpMode,
    #[ctor(into)]
    from: Mailbox,
    #[ctor(into)]
    #[validate(length(min = 1))]
    to: ArcVec<Mailbox>,
    #[ctor(into)]
    username: String,
    #[ctor(into)]
    password: RedactedString,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

        let creds = Credentials::new(self.username.clone(), self.password.inner.clone());

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
