# k-backup

Automated backup tool with encryption, compression, and retention management.

## Features

- **Scheduled Backups**: Cron-based automation (UTC)
- **Multiple Sources**: SQLite databases, file/directory globs, base64 content
- **Compression**: XZ (LZMA) with configurable levels and parallel threads
- **Encryption**: Age encryption (passphrase or recipients file)
- **Retention**: Grandfather-father-son rotation with safety nets
- **Notifications**: SMTP email and command hooks with event filtering
- **Event System**: Subscribe to backup_cycle_start, success, non_fatal_error, fatal_error
- **Failure Modes**: Per-notification on_failure: continue, skip, or error
- **Source Validation**: Pre-validates required sources before starting work
- **Streaming Pipeline**: Tar → Compress → Encrypt directly to disk (low memory)

## Quick Start

```bash
cargo build --release
./target/release/k_backup --config config.example.yml --once
```

## Configuration

See `config.example.yml` for a complete example. Key sections:

### File Sources

```yaml
files:
  - type: sqlite
    src: /path/to/db.sqlite3
    dst: db.sqlite3
    required: true           # fail cycle if this source errors (default)

  - type: glob
    src_dir: /path/to/files/
    globset: ["**/*.txt"]
    symlink_mode: follow     # follow|preserve|skip (default: follow)
    max_depth: 10            # recommended with follow mode
    required: false          # partial success ok for this source

  - type: base64
    content: "SGVsbG8="
    dst: hello.txt
```

### Notifications

```yaml
notifications:
  - name: my-hook
    type: command
    events: [success, fatal_error]
    on_failure: skip         # continue|skip|error
    command: ["/bin/notify"]
    stdin_json: true         # pipe event JSON to stdin
    env_inherit_mode: none   # none|all
    timeout: 60s
```

### Retention

```yaml
retention:
  default_retention: 7days
  daily_retention: 30days
  monthly_retention: 12months
  min_backups: 3
```

## Usage

```bash
# Run once and exit
k_backup --config config.yml --once

# Run as daemon (requires cron field in config)
k_backup --config config.yml

# Debug logging
RUST_LOG=debug k_backup --config config.yml --once
```

## Architecture

```
Source Files → TAR → XZ Compression → Age Encryption → Final Backup
```

Output filename: `{archive_base_name}.{timestamp}.tar[.xz][.age]`

## License

GPL-3.0
