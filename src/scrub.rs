use tokio::process::Command;
use tokio::time::{Duration, sleep};

use crate::config::EmailConfig;
use crate::email::send_mail;
use crate::error::AppError;
use crate::status::read_status_report;

#[derive(Debug, Clone, Default)]
struct ZpoolProperties {
    name: String,
}

impl ZpoolProperties {
    fn set_property(&mut self, prop: &str, value: &str) {
        if prop == "name" {
            self.name = value.to_string();
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ZpoolStatus {
    scan: String,
}

pub async fn execute_scrub(email_config: Option<EmailConfig>) -> Result<(), AppError> {
    let result = run_scrub(email_config.as_ref()).await;

    if let Err(e) = &result
        && let Some(ec) = email_config
    {
        let error_msg = format!("Error occurred: {}", e);
        send_mail(&ec, "ZFS Scrub Error", &error_msg).await?;
        println!("Error email sent");
    }

    result
}

async fn run_scrub(email_config: Option<&EmailConfig>) -> Result<(), AppError> {
    println!(
        "Start time: {}",
        jiff::Zoned::now().strftime("%Y-%m-%d %H:%M:%S %Z")
    );

    let zpools = get_zpool_list(&["name"]).await?;
    for props in &zpools {
        println!("Starting scrub for pool: {}", props.name);
        start_scrub(&props.name).await?;
    }

    while scrub_in_progress().await? {
        sleep(Duration::from_secs(5)).await;
    }

    println!("All scrubs complete");

    let report = read_status_report().await?;
    if report.is_healthy {
        println!("All pools are healthy");
    } else {
        println!("Unhealthy pools detected");

        if let Some(ec) = email_config {
            send_mail(ec, "ZFS Pool Unhealthy", &report.output).await?;
            println!("Email sent successfully");
        }
    }

    Ok(())
}

pub async fn scrub_in_progress() -> Result<bool, AppError> {
    let status = get_zpool_status().await?;
    Ok(status
        .iter()
        .any(|pool| pool.scan.contains("scrub in progress")))
}

async fn get_zpool_list(properties: &[&str]) -> Result<Vec<ZpoolProperties>, AppError> {
    let prop_list = properties.join(",");
    let output = Command::new("zpool")
        .args(["list", "-Ho", &prop_list])
        .output()
        .await
        .map_err(|e| AppError::Zpool(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Zpool(stderr.to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut zpools: Vec<ZpoolProperties> = Vec::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.is_empty() {
            continue;
        }

        let mut props = ZpoolProperties::default();

        for (i, prop) in properties.iter().enumerate() {
            if i < parts.len() {
                props.set_property(prop, parts[i]);
            }
        }

        if !props.name.is_empty() {
            zpools.push(props);
        }
    }

    Ok(zpools)
}

async fn get_zpool_status() -> Result<Vec<ZpoolStatus>, AppError> {
    let output = Command::new("zpool")
        .arg("status")
        .output()
        .await
        .map_err(|e| AppError::Zpool(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Zpool(stderr.to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut zpools: Vec<ZpoolStatus> = Vec::new();
    let mut current_pool: Option<ZpoolStatus> = None;

    for line in stdout.lines() {
        let trimmed = line.trim_start();

        if trimmed.starts_with("pool:") {
            if let Some(pool) = current_pool.take() {
                zpools.push(pool);
            }
            current_pool = Some(ZpoolStatus::default());
        } else if let Some(ref mut pool) = current_pool
            && trimmed.starts_with("scan:")
        {
            pool.scan = trimmed
                .strip_prefix("scan:")
                .unwrap_or("")
                .trim()
                .to_string();
        }
    }

    if let Some(pool) = current_pool {
        zpools.push(pool);
    }

    Ok(zpools)
}
async fn start_scrub(pool_name: &str) -> Result<(), AppError> {
    let output = Command::new("zpool")
        .args(["scrub", pool_name])
        .output()
        .await
        .map_err(|e| AppError::Zpool(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Zpool(stderr.to_string()));
    }

    Ok(())
}
