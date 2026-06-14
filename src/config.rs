use etcetera::BaseStrategy;
use serde::Deserialize;
use std::path::PathBuf;

use crate::error::AppError;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub email: Option<EmailConfig>,
    #[serde(default)]
    pub scrub: ScrubConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ScrubConfig {
    #[serde(default)]
    pub schedule: Option<ScheduleConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleConfig {
    pub cron: String,
    #[serde(default = "default_timezone")]
    pub timezone: String,
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

impl Config {
    pub fn daemon_schedule(&self) -> Result<&ScheduleConfig, AppError> {
        self.scrub.schedule.as_ref().ok_or_else(|| {
            AppError::ConfigFile(
                "Missing required configuration at [scrub.schedule] for daemon mode".to_string(),
            )
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

    #[test]
    fn parses_nested_scrub_schedule() {
        let config: Config = toml::from_str(
            r#"
            [scrub.schedule]
            cron = "15 3 * * 3"
            "#,
        )
        .expect("config should parse");

        let schedule = config.daemon_schedule().expect("schedule should exist");
        assert_eq!(schedule.cron, "15 3 * * 3");
        assert_eq!(schedule.timezone, "local");
    }

    #[test]
    fn daemon_mode_requires_schedule() {
        let config = Config::default();
        let err = config
            .daemon_schedule()
            .expect_err("schedule should be required");
        assert!(
            err.to_string()
                .contains("Missing required configuration at [scrub.schedule]")
        );
    }
}
