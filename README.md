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

- **Scheduled Backups**: Cron-based automation
- **Multiple Sources**: SQLite databases and file/directory patterns  
- **Compression**: XZ (LZMA) with parallel processing
- **Encryption**: Age encryption with passphrase support
- **Retention Management**: Configurable policies with grandfather-father-son rotation
- **Parallel Processing**: Multi-threaded operations for performance

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
     monthly_retention: 12months
     yearly_retention: 5years
   ```

3. **Run the backup daemon**:
   ```bash
   ./target/release/k_backup --config config.yml
   ```

## Configuration

### Scheduling
- **`cron`**: Standard cron expression for backup timing
  - `"0 1 * * *"` - Daily at 1:00 AM UTC
  - `"0 */6 * * *"` - Every 6 hours
  - `"0 2 * * 0"` - Weekly on Sunday at 2:00 AM UTC

**Note**: All times are in UTC, not local time.

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

### Compression
```yaml
compressor:
  compressor_type: xz
  level: 6        # optional: 0-9 (higher = smaller files, slower)
  thread: 4       # optional: parallel compression threads
```

### Encryption
```yaml
encryptor:
  encryptor_type: age
  secret_type: passphrase
  passphrase: 'your-secure-password'
```

**Security Note**: Passphrases are stored in plain text in config files. Consider using proper file permissions (600) and secure storage. Yes, this isn't ideal, but it's simple.

### Retention Policies
```yaml
retention:
  default_retention: 7days      # Base retention for all backups
  daily_retention: 30days       # Keep one backup per day for 30 days
  monthly_retention: 12months   # Keep one backup per month for 12 months
  yearly_retention: 5years      # Keep one backup per year for 5 years
```

Implements grandfather-father-son rotation: newer backups in each category are preserved even if they exceed the default retention period.

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
encryptor:
  encryptor_type: age
  secret_type: passphrase
  passphrase: 'secure-password-123'
compressor:
  compressor_type: xz
retention:
  default_retention: 30days
  monthly_retention: 12months
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
encryptor:
  encryptor_type: age
  secret_type: passphrase
  passphrase: 'server-backup-key'
compressor:
  compressor_type: xz
  level: 9
retention:
  default_retention: 7days
  daily_retention: 30days
  monthly_retention: 12months
  yearly_retention: 7years
```

## Known Limitations

- **UTC scheduling only** - Cron expressions run in UTC time (because timezones are hard)
- **Passphrase-only encryption** - Key file support not yet implemented
- **Error handling** - Some errors may cause the daemon to exit
- **No progress indicators** - Runs silently in background
- **Single instance** - No protection against multiple concurrent runs

## Troubleshooting

### Common Issues

1. **Permission denied**: Ensure read access to source files and write access to output directory
2. **Disk space**: Monitor backup directory; adjust retention policies as needed
3. **Time zone confusion**: Remember all schedules are UTC-based
4. **Process crashes**: Check logs and restart the daemon

### Debugging
Enable detailed logging:
```bash
RUST_LOG=debug ./k_backup --config config.yml
```

## Best Practices

- **Test your configuration** with a small dataset first
- **Verify backup integrity** by testing restore procedures
- **Monitor disk space** in your backup directory
- **Use appropriate file permissions** (600) for config files
- **Follow the 3-2-1 rule**: 3 copies, 2 different media, 1 offsite

## License

This project is licensed under the GNU General Public License v3.0 - see the LICENSE file for details.

---

*Built with Rust for reliable automated backups (it works on my machine™)*
