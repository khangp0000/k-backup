# k-backup

An automated backup tool with encryption, compression, and retention management.

## ⚠️ Important Notice

This is a personal project created for my own backup needs. While it works reliably for my use case, please consider the following:

- **Test thoroughly** before using in production (seriously, test it)
- **Verify backups** regularly to ensure they work
- **Have multiple backup strategies** - don't rely on any single tool
- **No warranty provided** - use at your own discretion

For enterprise environments, consider established solutions like Veeam, Bacula, or similar professional tools.

## Features

- **Scheduled Backups**: Cron-based automation with UTC scheduling
- **Multiple Sources**: SQLite databases, file/directory patterns, and base64 content
- **Compression**: XZ (LZMA) with configurable levels and parallel processing
- **Encryption**: Age encryption with passphrase support
- **Retention Management**: Sophisticated grandfather-father-son rotation with safety nets
- **Email Notifications**: SMTP notifications for backup failures and errors
- **Command Notifications**: Run arbitrary commands on backup events with JSON payload
- **Parallel Processing**: Multi-threaded operations for optimal performance
- **Robust Error Handling**: Non-fatal errors don't stop backups, comprehensive logging

## Quick Start

1. **Build the application**:
   ```bash
   cargo build --release
   ```

2. **Create a configuration file** (see `config.example.yml`):
   ```yaml
   cron: "0 1 * * *"  # Daily at 1 AM UTC
   archive_base_name: backup
   out_dir: ./backups/
   files:
     - type: sqlite
       src: /path/to/database.sqlite3
       dst: database.sqlite3
     - type: glob
       src_dir: /path/to/files/
       globset:
         - "**/*.txt"
         - "**/*.json"
     - type: base64
       content: "SGVsbG8gSSBsb3ZlIHU="
       dst: test.txt
   notifications:
     - type: smtp
       host: smtp.example.com
       smtp_mode: Ssl
       from: admin@example.com
       to: ["you@example.com"]
       username: username
       password: password
   encryptor:
     encryptor_type: age
     secret_type: passphrase
     passphrase: 'your-secure-password'
   compressor:
     compressor_type: xz
     level: 6        # optional: 0-9
     thread: 4       # optional: number of threads
   retention:
     default_retention: 7days
     daily_retention: 30days
     weekly_retention: 12weeks
     monthly_retention: 12months
     yearly_retention: 5years
     min_backups: 3  # safety net
   ```

3. **Run the backup daemon**:
   ```bash
   ./target/release/k_backup --config config.yml
   ```

   Or run a single backup and exit (for use with external schedulers):
   ```bash
   ./target/release/k_backup --config config.yml --once
   ```

   Or with debug logging:
   ```bash
   RUST_LOG=debug ./target/release/k_backup --config config.yml
   ```

## Configuration

### Scheduling
- **`cron`** *(optional)*: Standard cron expression for backup timing. If omitted, runs once and exits.
  - `"0 1 * * *"` - Daily at 1:00 AM UTC
  - `"0 */6 * * *"` - Every 6 hours
  - `"0 2 * * 0"` - Weekly on Sunday at 2:00 AM UTC

**Note**: All times are in UTC, not local time. Use `--once` or omit `cron` to run with external schedulers.

### File Sources

#### SQLite Databases
```yaml
- type: sqlite
  src: /var/lib/app/database.sqlite3
  dst: database.sqlite3
```
Uses SQLite's backup API for consistent snapshots even during active use.

#### File/Directory Patterns
```yaml
- type: glob
  src_dir: /home/user/
  dst_dir: user_files/  # optional: prefix within archive
  globset:
    - "Documents/**/*"
    - "Pictures/**/*.jpg"
    - "config/**/*"
```

#### Base64 Content
```yaml
- type: base64
  content: "SGVsbG8gSSBsb3ZlIHU="  # "Hello I love u" in base64
  dst: message.txt
```
Useful for including small configuration files or secrets directly in the backup.

### Compression
```yaml
compressor:
  compressor_type: xz
  level: 6        # optional: 0-9 (higher = smaller files, slower)
  thread: 4       # optional: parallel compression threads
```

### Encryption

#### Passphrase-based
```yaml
encryptor:
  encryptor_type: age
  secret_type: passphrase
  passphrase: 'your-secure-password'
```

#### Recipients file-based
```yaml
encryptor:
  encryptor_type: age
  secret_type: recipients_files
  recipients_files:
    - /path/to/recipients.txt
```

The recipients file contains one recipient per line (comments with `#` and empty lines are ignored). Supported recipient types:
- **age x25519**: `age1...` (native age public keys, generated with `age-keygen`)
- **SSH public keys**: `ssh-ed25519 AAAA...` or `ssh-rsa AAAA...`
- **Plugin recipients**: `age1pluginname1...` (requires `age-plugin-<name>` in `$PATH`, e.g., [age-plugin-yubikey](https://github.com/str4d/age-plugin-yubikey) for hardware tokens)

Multiple recipients can be listed — any corresponding private key can decrypt the backup. This is useful for shared access (e.g., team members each have their own key).

To generate a new age key pair:
```bash
age-keygen -o key.txt
# Public key: age1ql3z7hjy54pw3hyww5ayyfg7zqgvc7w3j2elw8zmrj2kg5sfn9aqmcac8p
```
Add the public key line to your recipients file. To decrypt, use `age -d -i key.txt`.

For more information, see the [age documentation](https://age-encryption.org) and the [rage](https://github.com/str4d/rage) Rust implementation.

**Security Note**: Passphrases are stored in plain text in config files. Consider using proper file permissions (600) and secure storage. Recipients file-based encryption avoids storing secrets in the config entirely — only public keys are referenced.

### Notifications

Notifications are event-driven. Each notification target subscribes to specific event types and defines failure behavior.

**Event types**: `backup_cycle_start`, `success`, `non_fatal_error`, `fatal_error`

**`on_failure`**: `continue` (default, log and proceed), `skip` (abort current cycle), `error` (stop daemon)

#### SMTP
```yaml
notifications:
  - name: email-ops
    type: smtp
    events: [non_fatal_error, fatal_error]
    on_failure: continue
    host: smtp.gmail.com
    smtp_mode: Ssl          # or StartTls
    from: backup@example.com
    to: ["admin@example.com", "ops@example.com"]
    username: backup@example.com
    password: app-password
```

#### Command
```yaml
  - name: backup-hook
    type: command
    events: [success, non_fatal_error, fatal_error]
    on_failure: skip
    command: ["/usr/local/bin/backup-notify", "--event-stdin"]
    stdin_json: true        # pipe event JSON to stdin (default: true)
    env_inherit_mode: none  # or "all" to inherit parent env
    env:                    # additional env vars (always applied)
      BACKUP_APP: "k-backup"
    timeout: 60s            # kill command after timeout (default: 30s)
```

When `env_inherit_mode: all`, use `env_inherit_allow` and `env_inherit_deny` to filter inherited variables.

### Retention Policies
```yaml
retention:
  default_retention: 7days      # Base retention for all backups
  daily_retention: 30days       # Keep one backup per day for 30 days
  weekly_retention: 12weeks     # Keep one backup per week for 12 weeks
  monthly_retention: 12months   # Keep one backup per month for 12 months
  yearly_retention: 5years      # Keep one backup per year for 5 years
  min_backups: 3               # Safety net - always keep at least this many
```

Implements sophisticated grandfather-father-son rotation:
- **Default retention**: Base policy applied to all backups
- **Daily/Weekly/Monthly/Yearly**: Preserves the most recent backup from each time period
- **Safety net**: `min_backups` prevents accidental deletion of all backups
- **Smart cleanup**: Only deletes backups that don't violate any retention rule

## How It Works

```
Source Files → TAR Archive → XZ Compression → Age Encryption → Final Backup
```

Backup files are named: `{archive_base_name}.{timestamp}.tar.xz.age`

Example: `backup.2025-11-20T15h39m11s_0000.tar.xz.age`

## Usage Examples

### Personal File Backup
```yaml
cron: "0 2 * * *"
archive_base_name: personal_backup
out_dir: /backups/personal/
files:
  - type: glob
    src_dir: /home/user/
    globset:
      - "Documents/**/*"
      - "Pictures/**/*"
      - ".ssh/**/*"
notifications:
  - type: smtp
    host: smtp.gmail.com
    smtp_mode: Ssl
    from: backup@example.com
    to: ["user@example.com"]
    username: backup@example.com
    password: app-password
encryptor:
  encryptor_type: age
  secret_type: passphrase
  passphrase: 'secure-password-123'
compressor:
  compressor_type: xz
retention:
  default_retention: 30days
  monthly_retention: 12months
  min_backups: 5
```

### Server Application Backup
```yaml
cron: "0 1 * * *"
archive_base_name: server_backup
out_dir: /var/backups/
files:
  - type: sqlite
    src: /var/lib/app/database.sqlite3
    dst: database.sqlite3
  - type: glob
    src_dir: /var/lib/app/
    globset:
      - "uploads/**/*"
      - "config/**/*"
      - "logs/*.log"
  - type: base64
    content: "$(cat /etc/app/secret.key | base64 -w 0)"
    dst: secret.key
notifications:
  - type: smtp
    host: smtp.company.com
    smtp_mode: StartTls
    from: server-backup@company.com
    to: ["ops@company.com", "admin@company.com"]
    username: server-backup@company.com
    password: secure-smtp-password
encryptor:
  encryptor_type: age
  secret_type: passphrase
  passphrase: 'server-backup-key'
compressor:
  compressor_type: xz
  level: 9
  thread: 8
retention:
  default_retention: 7days
  daily_retention: 30days
  monthly_retention: 12months
  yearly_retention: 7years
  min_backups: 10
```

## Known Limitations

- **UTC scheduling only** - Cron expressions run in UTC time (because timezones are hard)
- **No progress indicators** - Runs silently in background (check logs for details)
- **Single instance** - No protection against multiple concurrent runs
- **File permissions** - Backup process runs with current user permissions

## Troubleshooting

### Common Issues

1. **Permission denied**: Ensure read access to source files and write access to output directory
2. **Disk space**: Monitor backup directory; adjust retention policies as needed
3. **Time zone confusion**: Remember all schedules are UTC-based
4. **SMTP authentication**: Use app passwords for Gmail, check SMTP settings
5. **SQLite database locked**: Tool handles this gracefully using SQLite backup API
6. **Non-fatal errors**: Check email notifications and logs for file access issues

### Debugging
Enable detailed logging:
```bash
RUST_LOG=debug ./k_backup --config config.yml
```

For trace-level logging (very verbose):
```bash
RUST_LOG=trace ./k_backup --config config.yml
```

Log levels available: `error`, `warn`, `info`, `debug`, `trace`

## Best Practices

- **Test your configuration** with a small dataset first
- **Verify backup integrity** by testing restore procedures regularly
- **Monitor disk space** in your backup directory
- **Use appropriate file permissions** (600) for config files containing passwords
- **Set up email notifications** to catch backup issues early
- **Use strong passphrases** for encryption (consider using a password manager)
- **Test retention policies** to ensure they work as expected
- **Follow the 3-2-1 rule**: 3 copies, 2 different media, 1 offsite
- **Monitor logs** regularly for warnings and errors
- **Keep multiple backup strategies** - don't rely solely on k-backup

## License

This project is licensed under the GNU General Public License v3.0 - see the LICENSE file for details.

---

*Built with Rust for reliable automated backups (it works on my machine™)*

## Version History

- **v3.0.0**: Event-based notification system with command support, `on_failure` behavior (continue/skip/error), improved error reporting with `EntryErrors` grouping, error truncation (max 10)
- **v2.2.0**: Add recipients file-based encryption (x25519, SSH keys, age plugins), multi-recipient support
- **v2.1.0**: Add `--once` mode, weekly retention, feature flags for static builds, remove OpenSSL dependency, fix retention logic
- **v2.0.0**: Major refactor with builder patterns, email notifications, improved error handling, and comprehensive documentation
- **v1.x**: Initial releases with basic backup functionality
