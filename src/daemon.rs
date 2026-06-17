use chrono::Local;
use chrono_tz::Tz;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::RwLock;
use tokio_cron_scheduler::{Job, JobScheduler};
use uuid::Uuid;

use crate::config::{Config, ScheduleConfig, load_config_from_path};
use crate::error::AppError;
use crate::scrub::{execute_scrub, scrub_in_progress};
use crate::status::{SharedStatusNotificationState, execute_status_with_suppression};

#[derive(Debug, Clone)]
struct AppState {
    config: Arc<RwLock<Config>>,
    scrub_run_in_progress: Arc<AtomicBool>,
    status_run_in_progress: Arc<AtomicBool>,
    status_notification_state: Arc<SharedStatusNotificationState>,
}

#[derive(Debug, Clone, Copy)]
enum JobKind {
    Scrub,
    Status,
}

#[derive(Debug, Clone, Default)]
struct ActiveJobs {
    scrub: Option<Uuid>,
    status: Option<Uuid>,
}

struct RunGuard {
    flag: Arc<AtomicBool>,
}

impl RunGuard {
    fn try_acquire(flag: Arc<AtomicBool>) -> Option<Self> {
        match flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => Some(Self { flag }),
            Err(_) => None,
        }
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

pub async fn run_daemon(config_path: PathBuf) -> Result<(), AppError> {
    let initial_config = load_config_from_path(&config_path).await?;
    initial_config.validate_daemon_config()?;

    let scheduler = JobScheduler::new().await?;
    let state = Arc::new(AppState {
        config: Arc::new(RwLock::new(initial_config.clone())),
        scrub_run_in_progress: Arc::new(AtomicBool::new(false)),
        status_run_in_progress: Arc::new(AtomicBool::new(false)),
        status_notification_state: Arc::new(SharedStatusNotificationState::new()),
    });

    let mut active_jobs = install_jobs(&scheduler, &initial_config, state.clone()).await?;
    log_next_runs(&scheduler, &active_jobs).await;
    scheduler.start().await?;
    println!("zfshealth daemon started");

    let mut hup = signal(SignalKind::hangup())?;
    let mut term = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;

    loop {
        tokio::select! {
            _ = hup.recv() => {
                match reload_scheduler_config(&scheduler, &config_path, state.clone(), &active_jobs).await {
                    Ok(new_jobs) => {
                        active_jobs = new_jobs;
                        log_next_runs(&scheduler, &active_jobs).await;
                        println!("Configuration reloaded from {}", config_path.display());
                    }
                    Err(err) => {
                        eprintln!("Configuration reload failed: {}", err);
                    }
                }
            }
            _ = term.recv() => {
                println!("Received SIGTERM, shutting down");
                break;
            }
            _ = interrupt.recv() => {
                println!("Received SIGINT, shutting down");
                break;
            }
        }
    }

    let mut scheduler = scheduler;
    scheduler.shutdown().await?;
    Ok(())
}

async fn reload_scheduler_config(
    scheduler: &JobScheduler,
    config_path: &PathBuf,
    state: Arc<AppState>,
    active_jobs: &ActiveJobs,
) -> Result<ActiveJobs, AppError> {
    let new_config = load_config_from_path(config_path).await?;
    new_config.validate_daemon_config()?;
    let new_jobs = install_jobs(scheduler, &new_config, state.clone()).await?;

    remove_jobs(scheduler, active_jobs).await?;
    {
        let mut config = state.config.write().await;
        *config = new_config;
    }

    Ok(new_jobs)
}

async fn install_jobs(
    scheduler: &JobScheduler,
    config: &Config,
    state: Arc<AppState>,
) -> Result<ActiveJobs, AppError> {
    let mut active_jobs = ActiveJobs::default();

    if let Some(schedule) = config.scrub.schedule.as_ref() {
        let job = build_job(JobKind::Scrub, schedule, state.clone())?;
        active_jobs.scrub = Some(scheduler.add(job).await?);
    }

    if let Some(schedule) = config.status.schedule.as_ref() {
        let job = build_job(JobKind::Status, schedule, state.clone())?;
        active_jobs.status = Some(scheduler.add(job).await?);
    }

    Ok(active_jobs)
}

async fn remove_jobs(scheduler: &JobScheduler, active_jobs: &ActiveJobs) -> Result<(), AppError> {
    if let Some(job_id) = active_jobs.scrub {
        scheduler.remove(&job_id).await?;
    }
    if let Some(job_id) = active_jobs.status {
        scheduler.remove(&job_id).await?;
    }
    Ok(())
}

fn build_job(
    kind: JobKind,
    schedule: &ScheduleConfig,
    state: Arc<AppState>,
) -> Result<Job, AppError> {
    let cron = schedule.cron.trim();
    if cron.is_empty() {
        return Err(AppError::ConfigFile(format!(
            "{}.schedule.cron must not be empty",
            kind.label()
        )));
    }

    let cron = schedule.cron.clone();
    let timezone = schedule.timezone.trim().to_string();

    if timezone.eq_ignore_ascii_case("local") {
        Job::new_async_tz(&cron, Local, move |_job_id, _lock| {
            let state = state.clone();
            Box::pin(async move {
                if let Err(err) = run_scheduled_job(kind, state).await {
                    eprintln!("Scheduled {} failed: {}", kind.label(), err);
                }
            })
        })
        .map_err(|err| AppError::ConfigFile(format!("Invalid cron schedule: {}", err)))
    } else {
        let tz: Tz = timezone.parse().map_err(|_| {
            AppError::ConfigFile(format!(
                "Unsupported {}.schedule.timezone value: {}",
                kind.label(),
                schedule.timezone
            ))
        })?;

        Job::new_async_tz(&cron, tz, move |_job_id, _lock| {
            let state = state.clone();
            Box::pin(async move {
                if let Err(err) = run_scheduled_job(kind, state).await {
                    eprintln!("Scheduled {} failed: {}", kind.label(), err);
                }
            })
        })
        .map_err(|err| AppError::ConfigFile(format!("Invalid cron schedule: {}", err)))
    }
}

async fn log_next_runs(scheduler: &JobScheduler, active_jobs: &ActiveJobs) {
    if let Some(job_id) = active_jobs.scrub {
        log_next_run(scheduler, JobKind::Scrub, job_id).await;
    }
    if let Some(job_id) = active_jobs.status {
        log_next_run(scheduler, JobKind::Status, job_id).await;
    }
}

async fn log_next_run(scheduler: &JobScheduler, kind: JobKind, job_id: Uuid) {
    let mut scheduler = scheduler.clone();
    match scheduler.next_tick_for_job(job_id).await {
        Ok(Some(next)) => println!("Next {} run scheduled for {}", kind.label(), next),
        Ok(None) => println!("No next {} run scheduled", kind.label()),
        Err(err) => eprintln!("Failed to compute next {} run: {}", kind.label(), err),
    }
}

async fn run_scheduled_job(kind: JobKind, state: Arc<AppState>) -> Result<(), AppError> {
    match kind {
        JobKind::Scrub => run_scheduled_scrub(state).await,
        JobKind::Status => run_scheduled_status(state).await,
    }
}

async fn run_scheduled_scrub(state: Arc<AppState>) -> Result<(), AppError> {
    let Some(_guard) = RunGuard::try_acquire(state.scrub_run_in_progress.clone()) else {
        println!("Skipping scheduled scrub because a prior run is still in progress");
        return Ok(());
    };

    if scrub_in_progress().await? {
        println!("Skipping scheduled scrub because a scrub is already active");
        return Ok(());
    }

    let email_config = {
        let config = state.config.read().await;
        config.email.clone()
    };

    execute_scrub(email_config).await
}

async fn run_scheduled_status(state: Arc<AppState>) -> Result<(), AppError> {
    let Some(_guard) = RunGuard::try_acquire(state.status_run_in_progress.clone()) else {
        println!("Skipping scheduled status because a prior run is still in progress");
        return Ok(());
    };

    let (email_config, repeat_after) = {
        let config = state.config.read().await;
        (
            config.email.clone(),
            config
                .status
                .schedule
                .as_ref()
                .and_then(|schedule| schedule.repeat_after),
        )
    };

    execute_status_with_suppression(
        email_config.as_ref(),
        repeat_after,
        &state.status_notification_state,
    )
    .await
}

impl JobKind {
    fn label(self) -> &'static str {
        match self {
            JobKind::Scrub => "scrub",
            JobKind::Status => "status",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AppState, JobKind, build_job};
    use crate::config::{Config, ScheduleConfig};
    use crate::status::SharedStatusNotificationState;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tokio::sync::RwLock;

    fn state() -> Arc<AppState> {
        Arc::new(AppState {
            config: Arc::new(RwLock::new(Config::default())),
            scrub_run_in_progress: Arc::new(AtomicBool::new(false)),
            status_run_in_progress: Arc::new(AtomicBool::new(false)),
            status_notification_state: Arc::new(SharedStatusNotificationState::new()),
        })
    }

    #[test]
    fn rejects_invalid_scrub_timezone() {
        let schedule = ScheduleConfig {
            cron: "15 3 * * 3".to_string(),
            timezone: "Mars/Phobos".to_string(),
            repeat_after: None,
        };

        let err = match build_job(JobKind::Scrub, &schedule, state()) {
            Ok(_) => panic!("timezone should be rejected"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("Unsupported scrub.schedule.timezone value")
        );
    }

    #[test]
    fn rejects_invalid_status_timezone() {
        let schedule = ScheduleConfig {
            cron: "*/15 * * * *".to_string(),
            timezone: "Mars/Phobos".to_string(),
            repeat_after: None,
        };

        let err = match build_job(JobKind::Status, &schedule, state()) {
            Ok(_) => panic!("timezone should be rejected"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("Unsupported status.schedule.timezone value")
        );
    }

    #[test]
    fn rejects_empty_status_cron() {
        let schedule = ScheduleConfig {
            cron: "   ".to_string(),
            timezone: "local".to_string(),
            repeat_after: None,
        };

        let err = match build_job(JobKind::Status, &schedule, state()) {
            Ok(_) => panic!("empty cron should be rejected"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("status.schedule.cron must not be empty")
        );
    }
}
