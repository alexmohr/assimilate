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

    let subject = payload
        .get("event_type")
        .and_then(serde_json::Value::as_str)
        .map_or_else(
            || "Assimilate Notification".to_owned(),
            |t| format!("Assimilate: {t}"),
        );

    let body = serde_json::to_string_pretty(payload)?;

    let creds = Credentials::new(config.smtp_user.clone(), config.smtp_password.clone());

    let transport: AsyncSmtpTransport<Tokio1Executor> = match config.effective_security() {
        SmtpSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.smtp_host)
            .map_err(|e| NotificationError::Config(format!("smtp relay error: {e}")))?
            .port(config.smtp_port)
            .credentials(creds)
            .build(),
        SmtpSecurity::Starttls => {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.smtp_host)
                .map_err(|e| NotificationError::Config(format!("smtp starttls error: {e}")))?
                .port(config.smtp_port)
                .credentials(creds)
                .build()
        }
        SmtpSecurity::None => {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.smtp_host)
                .port(config.smtp_port)
                .credentials(creds)
                .build()
        }
    };

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
