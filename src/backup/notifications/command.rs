//! Command-based notification: pipes event JSON to a subprocess stdin.

use crate::backup::notifications::event::BackupEvent;
use crate::backup::result_error::error::Error;
use crate::backup::result_error::result::Result;
use bon::Builder;
use getset::Getters;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;
use validator::Validate;

/// Command notification configuration.
///
/// Spawns a subprocess and pipes the event as JSON to its stdin.
/// Non-zero exit is treated as notification failure.
#[derive(Clone, Debug, Serialize, Deserialize, Validate, Builder, Getters)]
#[serde(deny_unknown_fields)]
#[getset(get = "pub")]
pub struct CommandNotificationConfig {
    #[validate(length(min = 1))]
    #[builder(into)]
    command: Vec<String>,
    /// Whether to pipe event JSON to the command's stdin. Default: true.
    #[serde(default = "default_true")]
    #[builder(default = true)]
    stdin_json: bool,
    #[serde(default)]
    #[builder(default)]
    env_inherit_mode: EnvInheritMode,
    #[serde(default)]
    #[builder(default)]
    env_inherit_allow: Vec<String>,
    #[serde(default)]
    #[builder(default)]
    env_inherit_deny: Vec<String>,
    #[serde(default)]
    #[builder(default)]
    env: HashMap<String, String>,
    #[serde(default = "default_timeout", with = "humantime_serde")]
    #[builder(default = default_timeout())]
    timeout: Duration,
}

fn default_true() -> bool {
    true
}

fn default_timeout() -> Duration {
    Duration::from_secs(30)
}

/// Environment variable inheritance mode.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnvInheritMode {
    /// Inherit parent env filtered by allow/deny lists.
    All,
    /// Start with empty environment (default).
    #[default]
    None,
}

impl CommandNotificationConfig {
    pub fn send_event(&self, event: &BackupEvent) -> Result<()> {
        let mut cmd = Command::new(&self.command[0]);
        cmd.args(&self.command[1..]);
        cmd.stdin(if self.stdin_json {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Environment setup
        match &self.env_inherit_mode {
            EnvInheritMode::None => {
                cmd.env_clear();
            }
            EnvInheritMode::All => {
                cmd.env_clear();
                cmd.envs(std::env::vars().filter(|(k, _)| {
                    let allowed =
                        self.env_inherit_allow.is_empty() || self.env_inherit_allow.contains(k);
                    allowed && !self.env_inherit_deny.contains(k)
                }));
            }
        }
        cmd.envs(&self.env);

        let mut child = cmd.spawn().map_err(|e| {
            Error::from(std::io::Error::new(
                e.kind(),
                format!("Failed to spawn command {:?}: {}", self.command, e),
            ))
        })?;

        // Write JSON to stdin, then drop to close
        if self.stdin_json {
            if let Some(mut stdin) = child.stdin.take() {
                let json = serde_json::to_string(event).map_err(|e| {
                    Error::from(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
                })?;
                let _ = stdin.write_all(json.as_bytes());
            }
        }
        // Drop stdin to signal EOF before waiting
        drop(child.stdin.take());

        // Wait with timeout
        use wait_timeout::ChildExt;
        let timed_out = matches!(child.wait_timeout(self.timeout), Ok(None));
        if timed_out {
            let _ = child.kill();
        }

        // Read output after process exits (safe, no deadlock — process is done)
        let output = child.wait_with_output().map_err(Error::from)?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = stdout.trim_end();
        let stderr = stderr.trim_end();

        if !stdout.is_empty() {
            tracing::debug!("Command {:?} stdout: {}", self.command, stdout);
        }
        if !stderr.is_empty() {
            tracing::debug!("Command {:?} stderr: {}", self.command, stderr);
        }

        if timed_out {
            return Err(Error::from(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "Command {:?} timed out after {:?}\nstdout: {}\nstderr: {}",
                    self.command, self.timeout, stdout, stderr
                ),
            )));
        }

        if output.status.success() {
            Ok(())
        } else {
            Err(Error::from(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!(
                    "Command {:?} exited with status {}\nstdout: {}\nstderr: {}",
                    self.command, output.status, stdout, stderr
                ),
            )))
        }
    }
}
