//! Command notification target.

use crate::config::{CommandConfig, EnvInheritMode};
use crate::error::{CommandError, Result};
use crate::notifications::event::BackupEvent;
use std::io::Write;
use std::process::{Command, Stdio};
use wait_timeout::ChildExt;

pub fn send_event(config: &CommandConfig, event: &BackupEvent) -> Result<()> {
    let cmd_str = format!("{:?}", config.command);

    let mut cmd = Command::new(&config.command[0]);
    cmd.args(&config.command[1..]);
    cmd.stdin(if config.stdin_json {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    match config.env_inherit_mode {
        EnvInheritMode::None => {
            cmd.env_clear();
        }
        EnvInheritMode::All => {
            cmd.env_clear();
            cmd.envs(std::env::vars().filter(|(k, _)| {
                let allowed =
                    config.env_inherit_allow.is_empty() || config.env_inherit_allow.contains(k);
                allowed && !config.env_inherit_deny.contains(k)
            }));
        }
    }
    cmd.envs(&config.env);

    let mut child = cmd.spawn().map_err(|e| CommandError::Spawn {
        command: cmd_str.clone(),
        source: e,
    })?;

    if config.stdin_json {
        if let Some(mut stdin) = child.stdin.take() {
            let json = serde_json::to_string(event).map_err(CommandError::from)?;
            let _ = stdin.write_all(json.as_bytes());
        }
    }
    drop(child.stdin.take());

    let timed_out = matches!(child.wait_timeout(config.timeout), Ok(None));
    if timed_out {
        let _ = child.kill();
    }

    let output = child.wait_with_output().map_err(|e| CommandError::Wait {
        command: cmd_str.clone(),
        source: e,
    })?;

    let stdout =
        String::from_utf8_lossy(&output.stdout[..output.stdout.len().min(config.max_output_size)]);
    let stderr =
        String::from_utf8_lossy(&output.stderr[..output.stderr.len().min(config.max_output_size)]);
    let stdout = stdout.trim_end().to_string();
    let stderr = stderr.trim_end().to_string();

    if !stdout.is_empty() {
        tracing::debug!("Command {} stdout: {}", cmd_str, stdout);
    }
    if !stderr.is_empty() {
        tracing::debug!("Command {} stderr: {}", cmd_str, stderr);
    }

    if timed_out {
        return Err(CommandError::Timeout {
            command: cmd_str,
            timeout: config.timeout,
        }
        .into());
    }

    if !output.status.success() {
        return Err(CommandError::NonZeroExit {
            command: cmd_str,
            status: output.status.to_string(),
            stdout,
            stderr,
        }
        .into());
    }

    Ok(())
}
