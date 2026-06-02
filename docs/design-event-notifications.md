# Design: Event-Based Notification System

## Overview

Replace the current "notify on non-fatal error" approach with a typed event system.
Each backup lifecycle stage emits an event; notifications subscribe to event types they care about.

## Event Enum

```rust
#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackupEvent {
    BackupCycleStart {
        config: Arc<BackupConfig>,
        timestamp: DateTime<Utc>,
    },
    Success {
        config: Arc<BackupConfig>,
        timestamp: DateTime<Utc>,
        output_file: PathBuf,
    },
    NonFatalError {
        config: Arc<BackupConfig>,
        timestamp: DateTime<Utc>,
        output_file: PathBuf,
        errors: String, // Display output of EntryErrors
    },
    FatalError {
        config: Arc<BackupConfig>,
        timestamp: DateTime<Utc>,
        error: String, // Display output of Error
    },
}
```

- `config` is `Arc<BackupConfig>` — serialized as full config JSON (secrets redacted).
- `timestamp` is the cycle start time (`now`), same across all events in a cycle.
  Hooks can derive expected filename from `config + timestamp`.
- Errors serialize as their Display string (not structured, not deserializable).

## EventType Filter

```rust
#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    BackupCycleStart,
    Success,
    NonFatalError,
    FatalError,
}
```

## BackupConfig Signature Changes

`execute_backup_cycle`, `run_once`, and `start_loop` change from `&self` to `self: &Arc<Self>` to allow cloning into events without copying the config.

## Notification Config

Common fields are in a wrapper struct; type-specific fields use `#[serde(flatten)]`:

```rust
#[derive(Serialize, Deserialize)]
pub struct NotificationConfig {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "default_events")]
    pub events: Vec<EventType>,
    #[serde(default)]
    pub on_failure: OnFailure,
    #[serde(flatten)]
    pub target: NotificationTarget,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationTarget {
    Smtp(SmtpNotificationConfig),
    Command(CommandNotificationConfig),
}
```

- `name` — identifier for logging (optional, auto-generated if absent)
- `events` — list of `EventType` to subscribe to (default: `[non_fatal_error, fatal_error]`)
- `on_failure` — behavior when notification fails: `continue` (default), `skip`, or `error`

```yaml
notifications:
  - name: email-ops
    type: smtp
    events: [fatal_error, non_fatal_error]
    on_failure: continue
    host: smtp.gmail.com
    smtp_mode: Ssl
    from: admin@example.com
    to: ["ops@example.com"]
    username: user
    password: pass

  - name: backup-hook
    type: command
    events: [success, non_fatal_error, fatal_error, backup_cycle_start]
    on_failure: skip
    command: ["/usr/local/bin/backup-notify", "--event-stdin"]
    env_inherit_mode: all
    env_inherit_deny: ["SMTP_PASSWORD"]
    env:
      BACKUP_APP: "k-backup"
    timeout: 60s
```

## Command Notification

```rust
pub struct CommandNotificationConfig {
    pub command: Vec<String>, // [program, arg1, arg2, ...]
    #[serde(default)]
    pub env_inherit_mode: EnvInheritMode,
    #[serde(default)]
    pub env_inherit_allow: Vec<String>,
    #[serde(default)]
    pub env_inherit_deny: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "default_timeout", with = "humantime_serde")]
    pub timeout: Duration, // default: 30s
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EnvInheritMode {
    All,
    #[default]
    None,
}
```

Event JSON is piped to the command's stdin. Non-zero exit is treated as notification failure.

### Environment Variable Inheritance

- `env_inherit_mode: none` (default) → `env_clear()`, only `env` map is set.
- `env_inherit_mode: all` with empty `env_inherit_allow` → inherit all parent vars, minus `env_inherit_deny`.
- `env_inherit_mode: all` with `env_inherit_allow: [PATH, HOME]` → inherit only those, minus `env_inherit_deny`.
- `env` vars always applied on top regardless of mode.
- `env_inherit_allow`/`env_inherit_deny` are ignored when mode is `none`.

```yaml
# Clean env (default):
- name: hook
  type: command
  command: ["/bin/notify"]
  env:
    PATH: "/usr/bin"

# Inherit all except secrets:
- name: hook
  type: command
  command: ["/bin/notify"]
  env_inherit_mode: all
  env_inherit_deny: ["AWS_SECRET_ACCESS_KEY", "SMTP_PASSWORD"]

# Inherit only specific vars:
- name: hook
  type: command
  command: ["/bin/notify"]
  env_inherit_mode: all
  env_inherit_allow: ["PATH", "HOME"]
```

Implementation:
```rust
match self.env_inherit_mode {
    EnvInheritMode::None => { cmd.env_clear(); }
    EnvInheritMode::All => {
        cmd.env_clear();
        cmd.envs(std::env::vars().filter(|(k, _)| {
            let allowed = self.env_inherit_allow.is_empty() || self.env_inherit_allow.contains(k);
            allowed && !self.env_inherit_deny.contains(k)
        }));
    }
}
cmd.envs(&self.env);
```

Timeout: spawn child, wait with timeout, kill on expiry.

## Notification Target

`NotificationTarget` is an enum (not a trait), dispatched via `send_event`:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationTarget {
    Smtp(SmtpNotificationConfig),
    Command(CommandNotificationConfig),
}
```

Common fields (`name`, `events`, `on_failure`) live on the wrapper `NotificationConfig` struct.

## Dispatch

Sends to **all** subscribed notifications, collects failures, and returns the
highest-priority result (Error > Skip > Continue). Multiple errors of the same
priority are chained together.

```rust
fn dispatch_event(self: &Arc<Self>, event: &BackupEvent) -> Result<()> {
    let event_type = event.event_type();
    let mut worst: Option<(OnFailure, Error)> = None;

    for (i, notif) in self.notifications().iter().enumerate() {
        if notif.events.contains(&event_type) {
            if let Err(e) = notif.send_event(event) {
                let name = notif.display_name(i);
                let e = e.add_msg(format!("Notification '{}' failed", name));
                match notif.on_failure {
                    OnFailure::Continue => { tracing::error!("{} (continuing)", e); }
                    OnFailure::Skip => { /* upgrade worst to Skip, chain if same */ }
                    OnFailure::Error => { /* upgrade worst to Error, chain if same */ }
                }
            }
        }
    }

    match worst {
        None => Ok(()),
        Some((OnFailure::Skip, e)) => Err(Error::cycle_skipped(e)),
        Some((OnFailure::Error, e)) => Err(e),
        _ => unreachable!(),
    }
}
```

On fatal archive error, dispatch errors are chained with the archive error.
On non-fatal entry errors, dispatch errors are chained with the entry errors.

`display_name` returns `name` if set, otherwise `"{type}-{index}"` (e.g. `"smtp-0"`).

## Backward Compatibility

- If `events` is absent: default to `[non_fatal_error, fatal_error]` (current behavior).
- If `name` is absent: auto-generated as `"{type}-{index}"`.
- If `on_failure` is absent: default to `continue` (current behavior: log and proceed).
- SMTP `send_event` formats the event as subject + body text (same as current for error events).

## Migration Path

1. Add `BackupEvent` enum and `EventType`
2. Restructure `NotificationConfig` as wrapper with `#[serde(flatten)]` target
3. Add `name`, `events`, `on_failure` fields (with defaults)
4. Change `execute_backup_cycle`/`run_once`/`start_loop` to `self: &Arc<Self>`
5. `execute_backup_cycle` returns `Result<()>`; scheduling moved to `start_loop`
6. Implement `send_event` for SMTP (format event to subject + body)
7. Add `CommandNotificationConfig` with `send_event` (pipe JSON to stdin)
8. Add `CycleSkipped(Error)` variant; `start_loop` catches and continues
9. Replace current notification dispatch with `dispatch_event`
10. Update `main.rs` to wrap config in `Arc`
