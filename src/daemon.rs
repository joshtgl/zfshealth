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

#[derive(Debug, Clone)]
struct AppState {
    config: Arc<RwLock<Config>>,
    run_in_progress: Arc<AtomicBool>,
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
    let schedule = initial_config.daemon_schedule()?.clone();
    let scheduler = JobScheduler::new().await?;
    let state = Arc::new(AppState {
        config: Arc::new(RwLock::new(initial_config)),
        run_in_progress: Arc::new(AtomicBool::new(false)),
    });

    let job_id = install_scrub_job(&scheduler, &schedule, state.clone()).await?;
    log_next_run(&scheduler, job_id).await;
    scheduler.start().await?;
    println!("zfshealth daemon started");

    let mut hup = signal(SignalKind::hangup())?;
    let mut term = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut active_job_id = job_id;

    loop {
        tokio::select! {
            _ = hup.recv() => {
                match reload_scheduler_config(&scheduler, &config_path, state.clone(), active_job_id).await {
                    Ok(new_job_id) => {
                        active_job_id = new_job_id;
                        log_next_run(&scheduler, active_job_id).await;
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
    active_job_id: Uuid,
) -> Result<Uuid, AppError> {
    let new_config = load_config_from_path(config_path).await?;
    let new_schedule = new_config.daemon_schedule()?.clone();
    let new_job_id = install_scrub_job(scheduler, &new_schedule, state.clone()).await?;

    scheduler.remove(&active_job_id).await?;
    {
        let mut config = state.config.write().await;
        *config = new_config;
    }

    Ok(new_job_id)
}

async fn install_scrub_job(
    scheduler: &JobScheduler,
    schedule: &ScheduleConfig,
    state: Arc<AppState>,
) -> Result<Uuid, AppError> {
    let cron = schedule.cron.trim();
    if cron.is_empty() {
        return Err(AppError::ConfigFile(
            "scrub.schedule.cron must not be empty".to_string(),
        ));
    }

    let job = build_scrub_job(schedule, state)?;
    Ok(scheduler.add(job).await?)
}

fn build_scrub_job(schedule: &ScheduleConfig, state: Arc<AppState>) -> Result<Job, AppError> {
    let cron = schedule.cron.clone();
    let timezone = schedule.timezone.trim().to_string();

    if timezone.eq_ignore_ascii_case("local") {
        Job::new_async_tz(&cron, Local, move |_job_id, _lock| {
            let state = state.clone();
            Box::pin(async move {
                if let Err(err) = run_scheduled_scrub(state).await {
                    eprintln!("Scheduled scrub failed: {}", err);
                }
            })
        })
        .map_err(|err| AppError::ConfigFile(format!("Invalid cron schedule: {}", err)))
    } else {
        let tz: Tz = timezone.parse().map_err(|_| {
            AppError::ConfigFile(format!(
                "Unsupported scrub.schedule.timezone value: {}",
                schedule.timezone
            ))
        })?;

        Job::new_async_tz(&cron, tz, move |_job_id, _lock| {
            let state = state.clone();
            Box::pin(async move {
                if let Err(err) = run_scheduled_scrub(state).await {
                    eprintln!("Scheduled scrub failed: {}", err);
                }
            })
        })
        .map_err(|err| AppError::ConfigFile(format!("Invalid cron schedule: {}", err)))
    }
}

async fn log_next_run(scheduler: &JobScheduler, job_id: Uuid) {
    let mut scheduler = scheduler.clone();
    match scheduler.next_tick_for_job(job_id).await {
        Ok(Some(next)) => println!("Next scrub run scheduled for {}", next),
        Ok(None) => println!("No next scrub run scheduled"),
        Err(err) => eprintln!("Failed to compute next scrub run: {}", err),
    }
}

async fn run_scheduled_scrub(state: Arc<AppState>) -> Result<(), AppError> {
    let Some(_guard) = RunGuard::try_acquire(state.run_in_progress.clone()) else {
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

#[cfg(test)]
mod tests {
    use super::{AppState, build_scrub_job};
    use crate::config::{Config, ScheduleConfig};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tokio::sync::RwLock;

    #[test]
    fn rejects_invalid_timezone() {
        let schedule = ScheduleConfig {
            cron: "15 3 * * 3".to_string(),
            timezone: "Mars/Phobos".to_string(),
        };

        let state = Arc::new(AppState {
            config: Arc::new(RwLock::new(Config::default())),
            run_in_progress: Arc::new(AtomicBool::new(false)),
        });

        let err = match build_scrub_job(&schedule, state) {
            Ok(_) => panic!("timezone should be rejected"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("Unsupported scrub.schedule.timezone value")
        );
    }
}
