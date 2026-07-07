// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
    message::{Mailbox, MessageBuilder, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use serde::Deserialize;

use super::NotificationError;

/// SMTP security mode for email delivery.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SmtpSecurity {
    /// Unencrypted SMTP.
    None,
    /// STARTTLS upgrade on the standard port.
    #[default]
    Starttls,
    /// Implicit TLS on the standard port.
    Tls,
}

/// Configuration for an SMTP email notification channel.
#[derive(Debug, Deserialize)]
pub struct EmailConfig {
    /// SMTP server hostname.
    pub smtp_host: String,
    /// SMTP server port.
    pub smtp_port: u16,
    /// SMTP authentication username.
    pub smtp_user: String,
    /// SMTP authentication password.
    pub smtp_password: String,
    /// From-address for outgoing emails.
    pub from_address: String,
    /// Recipient addresses for the notification.
    pub to_addresses: Vec<String>,
    /// Security mode (None, Starttls, Tls).
    #[serde(default)]
    pub security: SmtpSecurity,
    /// Legacy flag; when true with Starttls security, forces Tls instead.
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

/// # Errors
///
/// Returns [`NotificationError::Config`] if the notification channel is misconfigured.
pub async fn send(
    config: &EmailConfig,
    payload: &serde_json::Value,
) -> Result<(), NotificationError> {
    let from: Mailbox = config
        .from_address
        .parse()
        .map_err(|e| NotificationError::Config(format!("invalid from address: {e}")))?;

    let subject = build_email_subject(payload);
    let body = build_email_body(payload);

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

/// # Errors
///
/// Returns [`NotificationError::Config`] if the notification channel is misconfigured.
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

pub(crate) fn build_email_subject(payload: &serde_json::Value) -> String {
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
    if matches!(
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
    }
}

pub(crate) fn build_email_body(payload: &serde_json::Value) -> String {
    let event_type = payload
        .get("event_type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let hostname = payload
        .get("hostname")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let repo_name = payload
        .get("repo_name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let schedule_name = payload
        .get("schedule_name")
        .and_then(serde_json::Value::as_str);
    let status = payload
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let timestamp = payload
        .get("timestamp")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let error_message = payload
        .get("error_message")
        .and_then(serde_json::Value::as_str);
    let archive_name = payload
        .get("archive_name")
        .and_then(serde_json::Value::as_str);

    let event_label = if event_type.is_empty() {
        "Notification".to_owned()
    } else {
        let label = event_type.replace('_', " ");
        let mut chars = label.chars();
        chars.next().map_or_else(String::new, |c| {
            c.to_uppercase().to_string() + chars.as_str()
        })
    };

    let mut parts = vec![format!("Event:      {event_label}")];
    if !hostname.is_empty() {
        parts.push(format!("Host:       {hostname}"));
    }
    if !repo_name.is_empty() {
        parts.push(format!("Repository: {repo_name}"));
    }
    if let Some(name) = schedule_name.filter(|n| !n.is_empty()) {
        parts.push(format!("Schedule:   {name}"));
    }
    if let Some(name) = archive_name {
        parts.push(format!("Archive:    {name}"));
    }
    if !status.is_empty() {
        parts.push(format!("Status:     {status}"));
    }
    if !timestamp.is_empty() {
        parts.push(format!("Time:       {timestamp}"));
    }
    if let Some(msg) = error_message {
        parts.push(String::new());
        parts.push(format!("Error:\n{msg}"));
    }

    parts.join("\n")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_backup_failed_includes_hostname_and_error() {
        let p = serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "web-server-01",
            "status": "failed",
            "error_message": "repository is locked",
        });
        assert_eq!(
            build_email_subject(&p),
            "Assimilate: backup failed - web-server-01: repository is locked"
        );
    }

    #[test]
    fn subject_backup_warning_includes_hostname_and_error() {
        let p = serde_json::json!({
            "event_type": "backup_warning",
            "hostname": "db-server-01",
            "status": "warning",
            "error_message": "quota exceeded",
        });
        assert_eq!(
            build_email_subject(&p),
            "Assimilate: backup warning - db-server-01: quota exceeded"
        );
    }

    #[test]
    fn subject_check_failed_includes_hostname_and_error() {
        let p = serde_json::json!({
            "event_type": "check_failed",
            "hostname": "myhost",
            "error_message": "integrity check failed",
        });
        assert_eq!(
            build_email_subject(&p),
            "Assimilate: check failed - myhost: integrity check failed"
        );
    }

    #[test]
    fn subject_backup_success_omits_error() {
        let p = serde_json::json!({
            "event_type": "backup_success",
            "hostname": "myhost",
            "error_message": "should be ignored",
        });
        assert_eq!(
            build_email_subject(&p),
            "Assimilate: backup success - myhost"
        );
    }

    #[test]
    fn subject_long_error_truncated() {
        let long_msg = "e".repeat(100);
        let p = serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "myhost",
            "error_message": long_msg,
        });
        let subject = build_email_subject(&p);
        assert!(subject.ends_with("..."));
        assert_eq!(
            subject,
            format!("Assimilate: backup failed - myhost: {}...", "e".repeat(60))
        );
    }

    #[test]
    fn subject_no_hostname_omits_dash() {
        let p = serde_json::json!({
            "event_type": "backup_failed",
            "error_message": "something went wrong",
        });
        assert_eq!(
            build_email_subject(&p),
            "Assimilate: backup failed: something went wrong"
        );
    }

    #[test]
    fn subject_empty_event_type_uses_notification() {
        let p = serde_json::json!({});
        assert_eq!(build_email_subject(&p), "Assimilate: Notification");
    }

    #[test]
    fn subject_no_error_message_for_failed_event() {
        let p = serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "myhost",
        });
        assert_eq!(
            build_email_subject(&p),
            "Assimilate: backup failed - myhost"
        );
    }

    #[test]
    fn body_backup_failed_includes_all_fields() {
        let p = serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "web-server-01",
            "repo_name": "server-daily",
            "schedule_name": "Nightly Server Backup",
            "archive_name": "web-server-01-2026-06-09T10:00:00.000000",
            "status": "failed",
            "timestamp": "2026-06-09T10:00:00Z",
            "error_message": "repository is locked",
        });
        let body = build_email_body(&p);
        assert!(body.contains("Event:      Backup failed"));
        assert!(body.contains("Host:       web-server-01"));
        assert!(body.contains("Repository: server-daily"));
        assert!(body.contains("Schedule:   Nightly Server Backup"));
        assert!(body.contains("Archive:    web-server-01-2026-06-09T10:00:00.000000"));
        assert!(body.contains("Status:     failed"));
        assert!(body.contains("Error:\nrepository is locked"));
        assert!(!body.contains('{'));
    }

    #[test]
    fn body_omits_schedule_when_absent() {
        let p = serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "web-server-01",
            "repo_name": "server-daily",
            "status": "failed",
            "timestamp": "2026-06-09T10:00:00Z",
        });
        let body = build_email_body(&p);
        assert!(!body.contains("Schedule:"));
    }

    #[test]
    fn body_agent_connected_omits_empty_fields() {
        let p = serde_json::json!({
            "event_type": "agent_connected",
            "hostname": "web-server-01",
            "repo_name": "",
            "status": "",
            "timestamp": "2026-06-09T10:00:00Z",
        });
        let body = build_email_body(&p);
        assert!(body.contains("Event:      Agent connected"));
        assert!(body.contains("Host:       web-server-01"));
        assert!(!body.contains("Repository:"));
        assert!(!body.contains("Status:"));
        assert!(!body.contains("Error:"));
    }

    #[test]
    fn body_empty_payload_returns_notification_label() {
        let body = build_email_body(&serde_json::json!({}));
        assert_eq!(body, "Event:      Notification");
    }
}
