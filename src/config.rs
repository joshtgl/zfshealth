use ::config::{Config as LayeredConfig, Environment, File};
use etcetera::BaseStrategy;
use jiff::{Span, SpanRelativeTo};
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::{self, Deserializer};
use std::path::PathBuf;
use std::time::Duration;

use crate::error::AppError;

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub email: Option<EmailConfig>,
    pub scrub: ScrubConfig,
    pub status: StatusConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ScrubConfig {
    #[serde(default)]
    pub schedule: Option<ScheduleConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct StatusConfig {
    #[serde(default)]
    pub schedule: Option<ScheduleConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleConfig {
    pub cron: String,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(default, deserialize_with = "deserialize_repeat_after")]
    pub repeat_after: Option<Duration>,
}

#[derive(Debug, Clone)]
pub struct EmailConfig {
    pub from: String,
    pub to: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: SecretString,
}

#[derive(Debug, Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    email: Option<RawEmailConfig>,
    #[serde(default)]
    scrub: ScrubConfig,
    #[serde(default)]
    status: StatusConfig,
}

#[derive(Debug, Deserialize, Default)]
struct RawEmailConfig {
    from: String,
    to: String,
    host: String,
    port: u16,
    username: String,
    password: Option<SecretString>,
    password_file: Option<PathBuf>,
}

fn default_timezone() -> String {
    "local".to_string()
}

fn deserialize_repeat_after<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    let Some(raw) = raw else {
        return Ok(None);
    };

    let span = raw.parse::<Span>().map_err(|err| {
        de::Error::custom(format!(
            "status.schedule.repeat_after must be a valid Jiff duration: {}",
            err
        ))
    })?;

    let signed_duration = span
        .to_duration(SpanRelativeTo::days_are_24_hours())
        .map_err(|err| {
            de::Error::custom(format!(
                "status.schedule.repeat_after could not be converted to an elapsed duration: {}",
                err
            ))
        })?;

    if signed_duration.is_negative() {
        return Err(de::Error::custom(
            "status.schedule.repeat_after must not be negative",
        ));
    }

    Duration::try_from(signed_duration)
        .map(Some)
        .map_err(|err| {
            de::Error::custom(format!(
                "status.schedule.repeat_after must be a non-negative elapsed duration: {}",
                err
            ))
        })
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            cron: String::new(),
            timezone: default_timezone(),
            repeat_after: None,
        }
    }
}

impl Config {
    async fn from_raw(raw: RawConfig) -> Result<Self, AppError> {
        Ok(Self {
            email: match raw.email {
                Some(email) => Some(EmailConfig::from_raw(email).await?),
                None => None,
            },
            scrub: raw.scrub,
            status: raw.status,
        })
    }

    pub fn validate_daemon_config(&self) -> Result<(), AppError> {
        if self.scrub.schedule.is_some() || self.status.schedule.is_some() {
            Ok(())
        } else {
            Err(AppError::ConfigFile(
                "Daemon mode requires at least one schedule under [scrub.schedule] or [status.schedule]".to_string(),
            ))
        }
    }
}

impl EmailConfig {
    async fn from_raw(raw: RawEmailConfig) -> Result<Self, AppError> {
        let password = match (raw.password, raw.password_file) {
            (Some(_), Some(_)) => {
                return Err(AppError::ConfigFile(
                    "email.password and email.password_file cannot both be set".to_string(),
                ));
            }
            (Some(password), None) => password,
            (None, Some(password_file)) => {
                let password = tokio::fs::read_to_string(&password_file)
                    .await
                    .map_err(|err| {
                        AppError::ConfigFile(format!(
                            "Could not read email.password_file {}: {}",
                            password_file.display(),
                            err
                        ))
                    })?;
                password.trim().to_owned().into()
            }
            (None, None) => {
                return Err(AppError::ConfigFile(
                    "email.password or email.password_file must be set when email is configured"
                        .to_string(),
                ));
            }
        };

        Ok(Self {
            from: raw.from,
            to: raw.to,
            host: raw.host,
            port: raw.port,
            username: raw.username,
            password,
        })
    }
}

pub fn resolve_config_path(arg_path: Option<PathBuf>) -> Result<Option<PathBuf>, AppError> {
    match arg_path {
        Some(path) => Ok(Some(path)),
        None => match etcetera::choose_base_strategy() {
            Ok(strategy) => {
                let config_file = strategy.config_dir().join("zfshealth").join("config.toml");
                if config_file.exists() {
                    Ok(Some(config_file))
                } else {
                    Ok(None)
                }
            }
            Err(_) => Ok(None),
        },
    }
}

pub async fn load_config(config_path: Option<&PathBuf>) -> Result<Config, AppError> {
    let raw = load_raw_config(config_path)?;
    Config::from_raw(raw).await
}

pub async fn load_config_from_path(config_path: &PathBuf) -> Result<Config, AppError> {
    if !config_path.exists() {
        return Err(AppError::ConfigFile(format!(
            "Configuration file not found: {}",
            config_path.display()
        )));
    }

    load_config(Some(config_path)).await
}

fn load_raw_config(config_path: Option<&PathBuf>) -> Result<RawConfig, AppError> {
    let mut builder = LayeredConfig::builder();

    if let Some(config_path) = config_path {
        builder = builder.add_source(File::from(config_path.as_path()));
    }

    builder = builder.add_source(
        Environment::with_prefix("ZFSHEALTH")
            .prefix_separator("_")
            .separator("__")
            .try_parsing(true),
    );

    Ok(builder.build()?.try_deserialize()?)
}

#[cfg(test)]
mod tests {
    use super::{Config, load_config, load_config_from_path};
    use secrecy::ExposeSecret;
    use std::ffi::OsStr;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::Duration;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[tokio::test]
    async fn parses_nested_scrub_schedule() {
        let _guard = env_guard();
        clear_zfshealth_env();
        let path = write_temp_file(
            "scrub-schedule",
            r#"
            [scrub.schedule]
            cron = "15 3 * * 3"
            "#,
        );

        let config = load_config_from_path(&path)
            .await
            .expect("config should parse");

        let schedule = config
            .scrub
            .schedule
            .as_ref()
            .expect("schedule should exist");
        assert_eq!(schedule.cron, "15 3 * * 3");
        assert_eq!(schedule.timezone, "local");
        assert_eq!(schedule.repeat_after, None);
    }

    #[tokio::test]
    async fn parses_status_repeat_after() {
        let _guard = env_guard();
        clear_zfshealth_env();
        let path = write_temp_file(
            "status-repeat-after",
            r#"
            [status.schedule]
            cron = "*/15 * * * *"
            repeat_after = "7d"
            "#,
        );

        let config = load_config_from_path(&path)
            .await
            .expect("config should parse");

        let schedule = config
            .status
            .schedule
            .as_ref()
            .expect("status schedule should exist");
        assert_eq!(
            schedule.repeat_after,
            Some(Duration::from_secs(7 * 24 * 60 * 60))
        );
    }

    #[tokio::test]
    async fn rejects_invalid_repeat_after() {
        let _guard = env_guard();
        clear_zfshealth_env();
        let path = write_temp_file(
            "invalid-repeat-after",
            r#"
            [status.schedule]
            cron = "*/15 * * * *"
            repeat_after = "not-a-duration"
            "#,
        );

        let err = load_config_from_path(&path)
            .await
            .expect_err("config should fail");

        assert!(
            err.to_string()
                .contains("status.schedule.repeat_after must be a valid Jiff duration")
        );
    }

    #[test]
    fn daemon_mode_requires_at_least_one_schedule() {
        let config = Config::default();
        let err = config
            .validate_daemon_config()
            .expect_err("schedule should be required");
        assert!(
            err.to_string()
                .contains("Daemon mode requires at least one schedule")
        );
    }

    #[tokio::test]
    async fn reads_inline_email_password() {
        let _guard = env_guard();
        clear_zfshealth_env();
        let path = write_temp_file(
            "inline-password",
            r#"
            [email]
            from = "zfshealth@example.com"
            to = "admin@example.com"
            host = "smtp.example.com"
            port = 587
            username = "smtp-user"
            password = "smtp-password"
            "#,
        );

        let config = load_config_from_path(&path)
            .await
            .expect("config should parse");
        let email = config.email.expect("email should exist");
        assert_eq!(email.password.expose_secret(), "smtp-password");
    }

    #[tokio::test]
    async fn reads_and_trims_email_password_file() {
        let _guard = env_guard();
        clear_zfshealth_env();
        let password_path = write_temp_file("smtp-password", "smtp-password\n");
        let config_path = write_temp_file(
            "password-file",
            &format!(
                r#"
                [email]
                from = "zfshealth@example.com"
                to = "admin@example.com"
                host = "smtp.example.com"
                port = 587
                username = "smtp-user"
                password_file = "{}"
                "#,
                password_path.display()
            ),
        );

        let config = load_config_from_path(&config_path)
            .await
            .expect("config should parse");
        let email = config.email.expect("email should exist");
        assert_eq!(email.password.expose_secret(), "smtp-password");
    }

    #[tokio::test]
    async fn rejects_email_with_both_password_sources() {
        let _guard = env_guard();
        clear_zfshealth_env();
        let password_path = write_temp_file("conflicting-smtp-password", "smtp-password\n");
        let config_path = write_temp_file(
            "conflicting-passwords",
            &format!(
                r#"
                [email]
                from = "zfshealth@example.com"
                to = "admin@example.com"
                host = "smtp.example.com"
                port = 587
                username = "smtp-user"
                password = "smtp-password"
                password_file = "{}"
                "#,
                password_path.display()
            ),
        );

        let err = load_config_from_path(&config_path)
            .await
            .expect_err("config should fail");
        assert!(
            err.to_string()
                .contains("email.password and email.password_file cannot both be set")
        );
    }

    #[tokio::test]
    async fn rejects_email_without_password_source() {
        let _guard = env_guard();
        clear_zfshealth_env();
        let config_path = write_temp_file(
            "missing-password",
            r#"
            [email]
            from = "zfshealth@example.com"
            to = "admin@example.com"
            host = "smtp.example.com"
            port = 587
            username = "smtp-user"
            "#,
        );

        let err = load_config_from_path(&config_path)
            .await
            .expect_err("config should fail");
        assert!(
            err.to_string()
                .contains("email.password or email.password_file must be set")
        );
    }

    #[tokio::test]
    async fn reports_missing_password_file() {
        let _guard = env_guard();
        clear_zfshealth_env();
        let config_path = write_temp_file(
            "missing-password-file",
            r#"
            [email]
            from = "zfshealth@example.com"
            to = "admin@example.com"
            host = "smtp.example.com"
            port = 587
            username = "smtp-user"
            password_file = "/tmp/zfshealth-missing-password-file"
            "#,
        );

        let err = load_config_from_path(&config_path)
            .await
            .expect_err("config should fail");
        assert!(
            err.to_string()
                .contains("Could not read email.password_file")
        );
    }

    #[tokio::test]
    async fn environment_overrides_file_values() {
        let _guard = env_guard();
        clear_zfshealth_env();
        set_env("ZFSHEALTH_EMAIL__HOST", "env.smtp.example.com");
        set_env("ZFSHEALTH_EMAIL__PASSWORD", "env-password");

        let config_path = write_temp_file(
            "env-overrides",
            r#"
            [email]
            from = "zfshealth@example.com"
            to = "admin@example.com"
            host = "file.smtp.example.com"
            port = 587
            username = "smtp-user"
            password = "file-password"
            "#,
        );

        let config = load_config_from_path(&config_path)
            .await
            .expect("config should parse");
        let email = config.email.expect("email should exist");
        assert_eq!(email.host, "env.smtp.example.com");
        assert_eq!(email.password.expose_secret(), "env-password");

        clear_zfshealth_env();
    }

    #[tokio::test]
    async fn environment_can_provide_nested_config_without_file() {
        let _guard = env_guard();
        clear_zfshealth_env();
        set_env("ZFSHEALTH_SCRUB__SCHEDULE__CRON", "15 3 * * 3");
        set_env("ZFSHEALTH_STATUS__SCHEDULE__CRON", "*/15 * * * *");
        set_env("ZFSHEALTH_STATUS__SCHEDULE__REPEAT_AFTER", "24h");
        set_env("ZFSHEALTH_EMAIL__FROM", "zfshealth@example.com");
        set_env("ZFSHEALTH_EMAIL__TO", "admin@example.com");
        set_env("ZFSHEALTH_EMAIL__HOST", "smtp.example.com");
        set_env("ZFSHEALTH_EMAIL__PORT", "587");
        set_env("ZFSHEALTH_EMAIL__USERNAME", "smtp-user");
        set_env("ZFSHEALTH_EMAIL__PASSWORD", "smtp-password");

        let config = load_config(None).await.expect("config should parse");

        assert_eq!(
            config
                .scrub
                .schedule
                .as_ref()
                .map(|schedule| &schedule.cron),
            Some(&"15 3 * * 3".to_string())
        );
        assert_eq!(
            config
                .status
                .schedule
                .as_ref()
                .map(|schedule| &schedule.cron),
            Some(&"*/15 * * * *".to_string())
        );
        assert_eq!(
            config
                .status
                .schedule
                .as_ref()
                .and_then(|schedule| schedule.repeat_after),
            Some(Duration::from_secs(24 * 60 * 60))
        );
        assert_eq!(
            config
                .email
                .as_ref()
                .map(|email| email.password.expose_secret()),
            Some("smtp-password")
        );

        clear_zfshealth_env();
    }

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn write_temp_file(name: &str, content: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("zfshealth-{}-{}.toml", name, std::process::id()));
        fs::write(&path, content).expect("temp file should be writable");
        path
    }

    fn set_env<K, V>(key: K, value: V)
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        // Tests serialize environment mutation with ENV_LOCK.
        unsafe {
            std::env::set_var(key, value);
        }
    }

    fn clear_zfshealth_env() {
        let keys: Vec<_> = std::env::vars()
            .map(|(key, _)| key)
            .filter(|key| key.starts_with("ZFSHEALTH_"))
            .collect();

        for key in keys {
            remove_env(key);
        }
    }

    fn remove_env<K: AsRef<OsStr>>(key: K) {
        // Tests serialize environment mutation with ENV_LOCK.
        unsafe {
            std::env::remove_var(key);
        }
    }
}
