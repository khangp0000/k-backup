//! Command notification target.

use crate::config::{CommandConfig, EnvInheritMode};
use crate::error::{CommandError, Result};
use crate::notifications::event::BackupEvent;
use std::io::{Read, Write};
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
            if let Err(e) = stdin.write_all(json.as_bytes()) {
                tracing::warn!("Failed to write event to command stdin: {}", e);
            }
        }
    }
    // Close stdin so child can proceed
    drop(child.stdin.take());

    // Read stdout/stderr concurrently (bounded), then wait for exit.
    // Pipes EOF when child exits, so reads complete naturally.
    // Timeout covers the entire operation.
    let max = config.max_output_size as u64;
    let timeout = config.timeout;

    let stderr_handle = std::thread::spawn({
        let mut stderr_pipe = child.stderr.take();
        move || {
            let mut buf = Vec::new();
            if let Some(ref mut s) = stderr_pipe {
                if let Err(e) = s.take(max).read_to_end(&mut buf) {
                    tracing::warn!("Failed to read command stderr: {}", e);
                }
            }
            buf
        }
    });

    let stdout_handle = std::thread::spawn({
        let mut stdout_pipe = child.stdout.take();
        move || {
            let mut buf = Vec::new();
            if let Some(ref mut s) = stdout_pipe {
                if let Err(e) = s.take(max).read_to_end(&mut buf) {
                    tracing::warn!("Failed to read command stdout: {}", e);
                }
            }
            buf
        }
    });

    // Wait with timeout — if child doesn't exit within timeout, kill it
    let status = match child.wait_timeout(timeout) {
        Ok(Some(status)) => status,
        Ok(None) => {
            if let Err(e) = child.kill() {
                tracing::error!("Failed to kill timed-out command {}: {}", cmd_str, e);
            }
            return Err(CommandError::Timeout {
                command: cmd_str,
                timeout,
            }
            .into());
        }
        Err(e) => {
            return Err(CommandError::Wait {
                command: cmd_str,
                source: e,
            }
            .into());
        }
    };

    let stdout_buf = stdout_handle.join().unwrap_or_default();
    let stderr_buf = stderr_handle.join().unwrap_or_default();

    let stdout = String::from_utf8_lossy(&stdout_buf).trim_end().to_string();
    let stderr = String::from_utf8_lossy(&stderr_buf).trim_end().to_string();

    if !stdout.is_empty() {
        tracing::debug!("Command {} stdout: {:?}", cmd_str, stdout);
    }
    if !stderr.is_empty() {
        tracing::debug!("Command {} stderr: {:?}", cmd_str, stderr);
    }

    if !status.success() {
        return Err(CommandError::NonZeroExit {
            command: cmd_str,
            status: status.to_string(),
            stdout,
            stderr,
        }
        .into());
    }

    Ok(())
}
