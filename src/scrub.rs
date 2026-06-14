use jiff::Zoned;
use tokio::process::Command;
use tokio::time::{Duration, sleep};

use crate::config::EmailConfig;
use crate::email::send_mail;
use crate::error::AppError;

#[derive(Debug, Clone, Default)]
struct ZpoolProperties {
    name: String,
    health: String,
}

impl ZpoolProperties {
    fn set_property(&mut self, prop: &str, value: &str) {
        match prop {
            "name" => self.name = value.to_string(),
            "health" => self.health = value.to_string(),
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ZpoolStatus {
    name: String,
    state: String,
    scan: String,
    scan_date: Option<Zoned>,
}

pub async fn execute_scrub(email_config: Option<EmailConfig>) -> Result<(), AppError> {
    let result = run_scrub(email_config.as_ref()).await;

    if let Err(e) = &result {
        if let Some(ec) = email_config {
            let error_msg = format!("Error occurred: {}", e);
            send_mail(&ec, "ZFS Scrub Error", &error_msg).await?;
            println!("Error email sent");
        }
    }

    result
}

async fn run_scrub(email_config: Option<&EmailConfig>) -> Result<(), AppError> {
    println!(
        "Start time: {}",
        jiff::Zoned::now().strftime("%Y-%m-%d %H:%M:%S %Z")
    );

    let zpools = get_zpool_list(&["name", "health"]).await?;
    for props in &zpools {
        println!("Starting scrub for pool: {}", props.name);
        start_scrub(&props.name).await?;
    }

    while scrub_in_progress().await? {
        sleep(Duration::from_secs(5)).await;
    }

    println!("All scrubs complete");

    let unhealthy = check_unhealthy().await?;
    if unhealthy {
        println!("Unhealthy pools detected");

        if let Some(ec) = email_config {
            let status_output = get_full_status().await?;
            send_mail(ec, "ZFS Pool Unhealthy", &status_output).await?;
            println!("Email sent successfully");
        }
    } else {
        println!("All pools are healthy");
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
            current_pool = Some(ZpoolStatus {
                name: trimmed
                    .strip_prefix("pool:")
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                ..Default::default()
            });
        } else if let Some(ref mut pool) = current_pool {
            if trimmed.starts_with("state:") {
                pool.state = trimmed
                    .strip_prefix("state:")
                    .unwrap_or("")
                    .trim()
                    .to_string();
            } else if trimmed.starts_with("scan:") {
                let scan_content = trimmed
                    .strip_prefix("scan:")
                    .unwrap_or("")
                    .trim()
                    .to_string();
                pool.scan = scan_content.clone();

                let date_str = if let Some(date_part) = scan_content.strip_prefix("on ") {
                    Some(date_part.to_string())
                } else if let Some(pos) = scan_content.find(" on ") {
                    Some(scan_content[pos + 4..].to_string())
                } else {
                    None
                };

                if let Some(ds) = date_str {
                    if let Some(ts) = parse_scan_date(&ds) {
                        pool.scan_date = Some(ts);
                    }
                }
            }
        }
    }

    if let Some(pool) = current_pool {
        zpools.push(pool);
    }

    Ok(zpools)
}

fn parse_scan_date(date_str: &str) -> Option<Zoned> {
    use jiff::civil::DateTime;

    match DateTime::strptime("%a %b %e %H:%M:%S %Y", date_str) {
        Ok(dt) => {
            let tz = jiff::tz::TimeZone::system();
            dt.to_zoned(tz).ok()
        }
        Err(_) => None,
    }
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

async fn check_unhealthy() -> Result<bool, AppError> {
    let output = Command::new("zpool")
        .args(["status", "-x"])
        .output()
        .await
        .map_err(|e| AppError::Zpool(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(!stdout.contains("all pools are healthy"))
}

async fn get_full_status() -> Result<String, AppError> {
    let output = Command::new("zpool")
        .arg("status")
        .output()
        .await
        .map_err(|e| AppError::Zpool(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Zpool(stderr.to_string()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
