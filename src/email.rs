use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use crate::config::EmailConfig;
use crate::error::AppError;

pub async fn send_mail(config: &EmailConfig, subject: &str, body: &str) -> Result<(), AppError> {
    let email = match lettre::Message::builder()
        .from(config.from.parse()?)
        .to(config.to.parse()?)
        .subject(subject)
        .body(body.to_string())
    {
        Ok(email) => email,
        Err(e) => return Err(AppError::Smtp(e.to_string())),
    };

    let credentials = Credentials::new(config.username.clone(), config.password.clone());
    let sender = build_mailer(config, credentials)?;

    sender.send(email).await?;
    sender.shutdown().await;
    Ok(())
}

fn build_mailer(
    config: &EmailConfig,
    credentials: Credentials,
) -> Result<AsyncSmtpTransport<Tokio1Executor>, AppError> {
    let builder = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
        .map_err(|e| AppError::Smtp(e.to_string()))?;

    Ok(builder.port(config.port).credentials(credentials).build())
}
