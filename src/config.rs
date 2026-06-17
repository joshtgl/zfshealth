use etcetera::BaseStrategy;
use jiff::{Span, SpanRelativeTo};
use serde::Deserialize;
use serde::de::{self, Deserializer};
use std::path::PathBuf;
use std::time::Duration;

use crate::error::AppError;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub email: Option<EmailConfig>,
    #[serde(default)]
    pub scrub: ScrubConfig,
    #[serde(default)]
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

#[derive(Debug, Clone, Deserialize, Default)]
pub struct EmailConfig {
    pub from: String,
    pub to: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
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
    match config_path {
        Some(path) => load_config_from_path(path).await,
        None => Ok(Config::default()),
    }
}

pub async fn load_config_from_path(config_path: &PathBuf) -> Result<Config, AppError> {
    if !config_path.exists() {
        return Err(AppError::ConfigFile(format!(
            "Configuration file not found: {}",
            config_path.display()
        )));
    }

    let config_content = tokio::fs::read_to_string(config_path).await?;
    Ok(toml::from_str(&config_content)?)
}

#[cfg(test)]
mod tests {
    use super::Config;
    use std::time::Duration;

    #[test]
    fn parses_nested_scrub_schedule() {
        let config: Config = toml::from_str(
            r#"
            [scrub.schedule]
            cron = "15 3 * * 3"
            "#,
        )
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

    #[test]
    fn parses_status_repeat_after() {
        let config: Config = toml::from_str(
            r#"
            [status.schedule]
            cron = "*/15 * * * *"
            repeat_after = "7d"
            "#,
        )
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

    #[test]
    fn rejects_invalid_repeat_after() {
        let err = toml::from_str::<Config>(
            r#"
            [status.schedule]
            cron = "*/15 * * * *"
            repeat_after = "not-a-duration"
            "#,
        )
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
}
