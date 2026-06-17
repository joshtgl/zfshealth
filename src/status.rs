use std::time::{Duration, Instant};

use tokio::process::Command;
use tokio::sync::Mutex;

use crate::config::EmailConfig;
use crate::email::send_mail;
use crate::error::AppError;

const HEALTHY_OUTPUT: &str = "all pools are healthy";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReport {
    pub output: String,
    pub is_healthy: bool,
}

#[derive(Debug, Clone)]
pub struct StatusNotificationState {
    last_unhealthy_output: Option<String>,
    last_email_sent_at: Option<Instant>,
}

impl StatusNotificationState {
    pub fn new() -> Self {
        Self {
            last_unhealthy_output: None,
            last_email_sent_at: None,
        }
    }
}

impl Default for StatusNotificationState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct SharedStatusNotificationState {
    inner: Mutex<StatusNotificationState>,
}

impl SharedStatusNotificationState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(StatusNotificationState::new()),
        }
    }
}

impl Default for SharedStatusNotificationState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotificationDecision {
    Skip,
    Send,
}

pub async fn execute_status(email_config: Option<EmailConfig>) -> Result<(), AppError> {
    let report = read_status_report().await?;
    log_status(&report);

    if !report.is_healthy
        && let Some(ec) = email_config
    {
        send_mail(&ec, "ZFS Pool Unhealthy", &report.output).await?;
        println!("Email sent successfully");
    }

    Ok(())
}

pub async fn execute_status_with_suppression(
    email_config: Option<&EmailConfig>,
    repeat_after: Option<Duration>,
    state: &SharedStatusNotificationState,
) -> Result<(), AppError> {
    let report = read_status_report().await?;
    log_status(&report);

    let should_send = {
        let mut guard = state.inner.lock().await;
        evaluate_notification(&report, repeat_after, &mut guard)
    };

    if should_send == NotificationDecision::Send
        && let Some(ec) = email_config
    {
        send_mail(ec, "ZFS Pool Unhealthy", &report.output).await?;
        {
            let mut guard = state.inner.lock().await;
            record_notification_sent(&report, &mut guard);
        }
        println!("Email sent successfully");
    }

    Ok(())
}

pub async fn read_status_report() -> Result<StatusReport, AppError> {
    let output = Command::new("zpool")
        .args(["status", "-x"])
        .output()
        .await
        .map_err(|e| AppError::Zpool(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Zpool(stderr.to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(StatusReport {
        is_healthy: stdout.contains(HEALTHY_OUTPUT),
        output: stdout,
    })
}

fn log_status(report: &StatusReport) {
    if report.is_healthy {
        println!("All pools are healthy");
    } else {
        println!("Unhealthy pools detected");
    }
}

fn evaluate_notification(
    report: &StatusReport,
    repeat_after: Option<Duration>,
    state: &mut StatusNotificationState,
) -> NotificationDecision {
    if report.is_healthy {
        state.last_unhealthy_output = None;
        state.last_email_sent_at = None;
        return NotificationDecision::Skip;
    }

    let output_changed = state.last_unhealthy_output.as_deref() != Some(report.output.as_str());
    let should_send = output_changed
        || match (repeat_after, state.last_email_sent_at) {
            (Some(interval), Some(last_sent_at)) => last_sent_at.elapsed() >= interval,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => true,
        };

    if should_send {
        NotificationDecision::Send
    } else {
        NotificationDecision::Skip
    }
}

fn record_notification_sent(report: &StatusReport, state: &mut StatusNotificationState) {
    state.last_unhealthy_output = Some(report.output.clone());
    state.last_email_sent_at = Some(Instant::now());
}

#[cfg(test)]
mod tests {
    use super::{
        NotificationDecision, StatusNotificationState, StatusReport, evaluate_notification,
        record_notification_sent,
    };
    use std::time::{Duration, Instant};

    fn unhealthy(output: &str) -> StatusReport {
        StatusReport {
            output: output.to_string(),
            is_healthy: false,
        }
    }

    fn healthy() -> StatusReport {
        StatusReport {
            output: "all pools are healthy".to_string(),
            is_healthy: true,
        }
    }

    #[test]
    fn first_unhealthy_result_sends() {
        let report = unhealthy("pool degraded");
        let mut state = StatusNotificationState::new();

        assert_eq!(
            evaluate_notification(&report, Some(Duration::from_secs(60)), &mut state),
            NotificationDecision::Send
        );
    }

    #[test]
    fn sent_unhealthy_result_updates_state() {
        let report = unhealthy("pool degraded");
        let mut state = StatusNotificationState::new();

        record_notification_sent(&report, &mut state);

        assert_eq!(state.last_unhealthy_output, Some(report.output));
        assert!(state.last_email_sent_at.is_some());
    }

    #[test]
    fn unsent_changed_output_keeps_retrying_without_repeat_after() {
        let report = unhealthy("pool faulted");
        let mut state = StatusNotificationState {
            last_unhealthy_output: Some("pool degraded".to_string()),
            last_email_sent_at: Some(Instant::now()),
        };

        assert_eq!(
            evaluate_notification(&report, None, &mut state),
            NotificationDecision::Send
        );
        assert_eq!(
            evaluate_notification(&report, None, &mut state),
            NotificationDecision::Send
        );
    }

    #[test]
    fn unchanged_output_without_repeat_after_stays_suppressed() {
        let report = unhealthy("pool degraded");
        let mut state = StatusNotificationState {
            last_unhealthy_output: Some(report.output.clone()),
            last_email_sent_at: Some(Instant::now() - Duration::from_secs(3600)),
        };

        assert_eq!(
            evaluate_notification(&report, None, &mut state),
            NotificationDecision::Skip
        );
    }

    #[test]
    fn unchanged_output_within_repeat_window_stays_suppressed() {
        let report = unhealthy("pool degraded");
        let mut state = StatusNotificationState {
            last_unhealthy_output: Some(report.output.clone()),
            last_email_sent_at: Some(Instant::now()),
        };

        assert_eq!(
            evaluate_notification(&report, Some(Duration::from_secs(3600)), &mut state),
            NotificationDecision::Skip
        );
    }

    #[test]
    fn unchanged_output_after_repeat_window_resends() {
        let report = unhealthy("pool degraded");
        let mut state = StatusNotificationState {
            last_unhealthy_output: Some(report.output.clone()),
            last_email_sent_at: Some(Instant::now() - Duration::from_secs(7200)),
        };

        assert_eq!(
            evaluate_notification(&report, Some(Duration::from_secs(3600)), &mut state),
            NotificationDecision::Send
        );
    }

    #[test]
    fn changed_output_resends_immediately() {
        let report = unhealthy("pool faulted");
        let mut state = StatusNotificationState {
            last_unhealthy_output: Some("pool degraded".to_string()),
            last_email_sent_at: Some(Instant::now()),
        };

        assert_eq!(
            evaluate_notification(&report, None, &mut state),
            NotificationDecision::Send
        );
    }

    #[test]
    fn healthy_result_clears_state() {
        let mut state = StatusNotificationState {
            last_unhealthy_output: Some("pool degraded".to_string()),
            last_email_sent_at: Some(Instant::now()),
        };

        assert_eq!(
            evaluate_notification(&healthy(), Some(Duration::from_secs(60)), &mut state),
            NotificationDecision::Skip
        );
        assert_eq!(state.last_unhealthy_output, None);
        assert_eq!(state.last_email_sent_at, None);
    }
}
