# zfshealth

`zfshealth` is a ZFS scrub scheduler and health notifier. The packaged service runs as a systemd-managed daemon and reads its configuration from `/etc/zfshealth/config.toml`.

## Install

Install the Debian package:

```bash
sudo apt install ./zfshealth_<version>_amd64.deb
```

After installation, manage the service with systemd:

```bash
sudo systemctl status zfshealth-scrub.service
sudo systemctl restart zfshealth-scrub.service
sudo systemctl reload zfshealth-scrub.service
```

## Use

Run a scrub immediately:

```bash
zfshealth run scrub --config /etc/zfshealth/config.toml
```

Run a status check immediately:

```bash
zfshealth run status --config /etc/zfshealth/config.toml
```

Run the daemon manually in the foreground:

```bash
zfshealth daemon --config /etc/zfshealth/config.toml
```

Reload configuration for the packaged service without restarting it:

```bash
sudo systemctl reload zfshealth-scrub.service
```

## Configure

The packaged service uses:

`/etc/zfshealth/config.toml`

For non-packaged manual runs, the default config path is:

`~/.config/zfshealth/config.toml`

Minimal daemon configuration with both jobs:

```toml
[scrub.schedule]
cron = "15 3 * * 3"

[status.schedule]
cron = "*/15 * * * *"
repeat_after = "24h"
```

Optional timezone:

```toml
[scrub.schedule]
cron = "15 3 * * 3"
timezone = "local"

[status.schedule]
cron = "*/15 * * * *"
timezone = "local"
repeat_after = "24h"
```

Email notifications are optional. When configured, `zfshealth` sends mail for scrub errors and unhealthy pool status:

```toml
[scrub.schedule]
cron = "15 3 * * 3"

[status.schedule]
cron = "*/15 * * * *"
repeat_after = "24h"

[email]
from = "zfshealth@example.com"
to = "admin@example.com"
host = "smtp.example.com"
port = 587
username = "smtp-user"
password_file = "/etc/zfshealth/smtp-password"
```

Inline `password = "smtp-password"` is still supported, but `password_file` is preferred so secrets can be kept out of the main configuration file.

Any configuration value can be overridden with environment variables using the `ZFSHEALTH` prefix and double underscores for nested tables:

```bash
ZFSHEALTH_EMAIL__HOST=smtp.example.com
ZFSHEALTH_EMAIL__PORT=587
ZFSHEALTH_EMAIL__USERNAME=smtp-user
ZFSHEALTH_EMAIL__PASSWORD_FILE=/etc/zfshealth/smtp-password
ZFSHEALTH_EMAIL__PASSWORD=smtp-password
ZFSHEALTH_STATUS__SCHEDULE__CRON="*/15 * * * *"
```

The `cron` value uses standard 5-field cron syntax:

- minute
- hour
- day of month
- month
- day of week

Example:

- `15 3 * * 3` runs every Wednesday at 03:15
- `*/15 * * * *` runs every 15 minutes

`status.schedule.repeat_after` is optional and uses Jiff-friendly duration strings such as `60s`, `15m`, `24h`, or `7d`. If omitted, `zfshealth` only resends unhealthy status email when the `zpool status -x` output changes.
