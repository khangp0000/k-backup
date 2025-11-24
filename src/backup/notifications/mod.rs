use crate::backup::notifications::smtp::SmtpNotificationConfig;
use crate::backup::result_error::result::Result;
use derive_ctor::ctor;
use derive_more::From;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::result;
use validator::{Validate, ValidationErrors};

pub mod smtp;

#[derive(Clone, From, Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
#[derive(ctor)]
#[ctor(prefix = new, vis = pub)]
pub enum NotificationConfig {
    Smtp(#[ctor(into)] SmtpNotificationConfig),
}

impl Validate for NotificationConfig {
    fn validate(&self) -> result::Result<(), ValidationErrors> {
        match self {
            Self::Smtp(inner) => inner.validate(),
        }
    }
}

impl Notification for NotificationConfig {
    fn send<D1: Display, D2: Display>(&self, topic: D1, msg: D2) -> Result<()> {
        match self {
            Self::Smtp(inner) => inner.send(topic, msg),
        }
    }
}

pub trait Notification {
    fn send<D1: Display, D2: Display>(&self, topic: D1, msg: D2) -> Result<()>;
}
