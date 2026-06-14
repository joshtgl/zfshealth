use std::io;

#[derive(Debug)]
pub enum AppError {
    Io(io::Error),
    Join(tokio::task::JoinError),
    Toml(toml::de::Error),
    EmailParse(lettre::address::AddressError),
    Scheduler(String),
    Smtp(String),
    ConfigFile(String),
    Zpool(String),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Io(e) => write!(f, "IO error: {}", e),
            AppError::Join(e) => write!(f, "Task join error: {}", e),
            AppError::Toml(e) => write!(f, "TOML parsing error: {}", e),
            AppError::EmailParse(e) => write!(f, "Email address error: {}", e),
            AppError::Scheduler(e) => write!(f, "Scheduler error: {}", e),
            AppError::Smtp(e) => write!(f, "SMTP error: {}", e),
            AppError::ConfigFile(msg) => write!(f, "Configuration file error: {}", msg),
            AppError::Zpool(msg) => write!(f, "zpool error: {}", msg),
        }
    }
}

impl std::error::Error for AppError {}

impl From<io::Error> for AppError {
    fn from(err: io::Error) -> Self {
        AppError::Io(err)
    }
}

impl From<tokio::task::JoinError> for AppError {
    fn from(err: tokio::task::JoinError) -> Self {
        AppError::Join(err)
    }
}

impl From<toml::de::Error> for AppError {
    fn from(err: toml::de::Error) -> Self {
        AppError::Toml(err)
    }
}

impl From<lettre::address::AddressError> for AppError {
    fn from(err: lettre::address::AddressError) -> Self {
        AppError::EmailParse(err)
    }
}

impl From<lettre::transport::smtp::Error> for AppError {
    fn from(err: lettre::transport::smtp::Error) -> Self {
        AppError::Smtp(err.to_string())
    }
}

impl From<tokio_cron_scheduler::JobSchedulerError> for AppError {
    fn from(err: tokio_cron_scheduler::JobSchedulerError) -> Self {
        AppError::Scheduler(err.to_string())
    }
}
