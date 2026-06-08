// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
    message::{Mailbox, MessageBuilder, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use serde::Deserialize;

use super::NotificationError;

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SmtpSecurity {
    None,
    #[default]
    Starttls,
    Tls,
}

#[derive(Debug, Deserialize)]
pub struct EmailConfig {
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_user: String,
    pub smtp_password: String,
    pub from_address: String,
    pub to_addresses: Vec<String>,
    #[serde(default)]
    pub security: SmtpSecurity,
    #[serde(default)]
    pub use_tls: bool,
}

impl EmailConfig {
    fn effective_security(&self) -> SmtpSecurity {
        if self.security != SmtpSecurity::Starttls {
            return self.security;
        }
        if self.use_tls {
            SmtpSecurity::Tls
        } else {
            SmtpSecurity::Starttls
        }
    }
}

pub async fn send(
    config: &EmailConfig,
    payload: &serde_json::Value,
) -> Result<(), NotificationError> {
    let from: Mailbox = config
        .from_address
        .parse()
        .map_err(|e| NotificationError::Config(format!("invalid from address: {e}")))?;

    let event_type_str = payload
        .get("event_type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let hostname = payload
        .get("hostname")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let event_label = if event_type_str.is_empty() {
        "Notification".to_owned()
    } else {
        event_type_str.replace('_', " ")
    };
    let base = if hostname.is_empty() {
        format!("Assimilate: {event_label}")
    } else {
        format!("Assimilate: {event_label} - {hostname}")
    };
    let subject = if matches!(
        event_type_str,
        "backup_warning" | "backup_failed" | "check_failed"
    ) {
        if let Some(msg) = payload
            .get("error_message")
            .and_then(serde_json::Value::as_str)
        {
            let mut chars = msg.chars();
            let short: String = chars.by_ref().take(60).collect();
            let short = if chars.next().is_some() {
                format!("{short}...")
            } else {
                short
            };
            format!("{base}: {short}")
        } else {
            base
        }
    } else {
        base
    };

    let body = serde_json::to_string_pretty(payload)?;

    let creds = Credentials::new(config.smtp_user.clone(), config.smtp_password.clone());

    let transport = build_transport(
        &config.smtp_host,
        config.smtp_port,
        config.effective_security(),
        creds,
    )?;

    for to_addr in &config.to_addresses {
        let to: Mailbox = to_addr
            .parse()
            .map_err(|e| NotificationError::Config(format!("invalid to address: {e}")))?;

        let message = MessageBuilder::new()
            .from(from.clone())
            .to(to)
            .subject(&subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body.clone())
            .map_err(|e| NotificationError::Config(format!("failed to build email: {e}")))?;

        transport.send(message).await?;
    }

    Ok(())
}

pub async fn validate_credentials(
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    security: SmtpSecurity,
) -> Result<(), NotificationError> {
    let creds = Credentials::new(user.to_owned(), password.to_owned());
    let transport = build_transport(host, port, security, creds)?;
    transport
        .test_connection()
        .await
        .map_err(|e| NotificationError::Config(format!("SMTP login failed: {e}")))?;
    Ok(())
}

fn build_transport(
    host: &str,
    port: u16,
    security: SmtpSecurity,
    creds: Credentials,
) -> Result<AsyncSmtpTransport<Tokio1Executor>, NotificationError> {
    let transport = match security {
        SmtpSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(host)
            .map_err(|e| NotificationError::Config(format!("smtp relay error: {e}")))?
            .port(port)
            .credentials(creds)
            .build(),
        SmtpSecurity::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
            .map_err(|e| NotificationError::Config(format!("smtp starttls error: {e}")))?
            .port(port)
            .credentials(creds)
            .build(),
        SmtpSecurity::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
            .port(port)
            .credentials(creds)
            .build(),
    };
    Ok(transport)
}
