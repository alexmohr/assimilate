// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
    message::{Mailbox, MessageBuilder, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use serde::Deserialize;

use super::{
    NotificationError,
    template::{format_bytes, format_duration_secs, render_template},
};

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
    /// This channel's own title (subject line) template, in place of the fixed `event: host /
    /// repo` default. Independent of every other channel's template -- see
    /// [`super::template::render_template`] for the placeholder syntax.
    #[serde(default)]
    pub title_template: Option<String>,
    /// This channel's own body template, in place of the fixed field-by-field default.
    /// Independent of every other channel's template -- see
    /// [`super::template::render_template`] for the placeholder syntax.
    #[serde(default)]
    pub body_template: Option<String>,
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

/// Resolves this channel's subject and body: its own `title_template`/`body_template` when
/// set, falling back to [`build_email_subject`]/[`build_email_body`]'s fixed defaults for a
/// channel created before this feature existed.
fn resolve_subject_and_body(config: &EmailConfig, payload: &serde_json::Value) -> (String, String) {
    let subject = config.title_template.as_deref().map_or_else(
        || build_email_subject(payload),
        |tpl| render_template(tpl, payload),
    );
    let body = config.body_template.as_deref().map_or_else(
        || build_email_body(payload),
        |tpl| render_template(tpl, payload),
    );
    (subject, body)
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

    let (subject, body) = resolve_subject_and_body(config, payload);

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
        .filter(|s| !s.is_empty());
    let repo_name = payload
        .get("repo_name")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty());
    let label = super::event_label(event_type_str);

    match (hostname, repo_name) {
        (Some(hostname), Some(repo_name)) => format!("{label}: {hostname} / {repo_name}"),
        (Some(hostname), None) => format!("{label}: {hostname}"),
        (None, _) => label.to_owned(),
    }
}

/// Fields pulled out of a notification payload for [`build_email_body`], gathered up front
/// so the body-assembly logic below reads as a flat list of "if present, add this line"
/// rather than being interleaved with `payload.get(...)` boilerplate.
struct EmailBodyFields<'a> {
    event_type: &'a str,
    hostname: &'a str,
    repo_name: &'a str,
    schedule_name: Option<&'a str>,
    next_run_at: Option<&'a str>,
    timestamp: &'a str,
    error_message: Option<&'a str>,
    archive_name: Option<&'a str>,
    duration_secs: Option<i64>,
    original_size: Option<i64>,
    compressed_size: Option<i64>,
    deduplicated_size: Option<i64>,
    files_processed: Option<i64>,
    warnings: Vec<&'a str>,
    activity_url: Option<&'a str>,
}

impl<'a> EmailBodyFields<'a> {
    fn from_payload(payload: &'a serde_json::Value) -> Self {
        let str_field = |key: &str| payload.get(key).and_then(serde_json::Value::as_str);
        let int_field = |key: &str| payload.get(key).and_then(serde_json::Value::as_i64);
        Self {
            event_type: str_field("event_type").unwrap_or(""),
            hostname: str_field("hostname").unwrap_or(""),
            repo_name: str_field("repo_name").unwrap_or(""),
            schedule_name: str_field("schedule_name"),
            next_run_at: str_field("next_run_at"),
            timestamp: str_field("timestamp").unwrap_or(""),
            error_message: str_field("error_message"),
            archive_name: str_field("archive_name"),
            duration_secs: int_field("duration_secs"),
            original_size: int_field("original_size"),
            compressed_size: int_field("compressed_size"),
            deduplicated_size: int_field("deduplicated_size"),
            files_processed: int_field("files_processed"),
            warnings: payload
                .get("warnings")
                .and_then(serde_json::Value::as_array)
                .map(|arr| arr.iter().filter_map(serde_json::Value::as_str).collect())
                .unwrap_or_default(),
            activity_url: str_field("activity_url"),
        }
    }
}

/// Renders the `Size:` line (with an optional "new" suffix from deduplication), or `None`
/// when the payload doesn't carry both an original and compressed size.
fn format_size_line(
    original: Option<i64>,
    compressed: Option<i64>,
    dedup: Option<i64>,
) -> Option<String> {
    let (original, compressed) = (original?, compressed?);
    let dedup_suffix = dedup.map_or_else(String::new, |dedup| {
        format!(" ({} new)", format_bytes(dedup))
    });
    Some(format!(
        "Size:        {} -> {} compressed{dedup_suffix}",
        format_bytes(original),
        format_bytes(compressed),
    ))
}

pub(crate) fn build_email_body(payload: &serde_json::Value) -> String {
    let fields = EmailBodyFields::from_payload(payload);
    let event_label = super::event_label(fields.event_type);

    let mut parts = vec![format!("Event:       {event_label}")];
    if !fields.hostname.is_empty() {
        parts.push(format!("Host:        {}", fields.hostname));
    }
    if !fields.repo_name.is_empty() {
        parts.push(format!("Repository:  {}", fields.repo_name));
    }
    if let Some(name) = fields.schedule_name.filter(|n| !n.is_empty()) {
        match fields.next_run_at {
            Some(next) => parts.push(format!("Schedule:    {name} (next run: {next})")),
            None => parts.push(format!("Schedule:    {name}")),
        }
    }
    if let Some(name) = fields.archive_name {
        parts.push(format!("Archive:     {name}"));
    }
    if let Some(secs) = fields.duration_secs {
        parts.push(format!("Duration:    {}", format_duration_secs(secs)));
    }
    if let Some(size_line) = format_size_line(
        fields.original_size,
        fields.compressed_size,
        fields.deduplicated_size,
    ) {
        parts.push(size_line);
    }
    if let Some(files) = fields.files_processed {
        parts.push(format!("Files:       {files} processed"));
    }
    if !fields.timestamp.is_empty() {
        parts.push(format!("Time:        {}", fields.timestamp));
    }
    if !fields.warnings.is_empty() {
        parts.push(String::new());
        parts.push("Warnings:".to_owned());
        parts.extend(fields.warnings.iter().map(|w| format!("- {w}")));
    }
    if let Some(msg) = fields.error_message {
        parts.push(String::new());
        parts.push(format!("Error:\n{msg}"));
    }
    if let Some(url) = fields.activity_url {
        parts.push(String::new());
        parts.push(format!("View activity log: {url}"));
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
    fn subject_includes_hostname_and_repo() {
        let p = serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "web-server-01",
            "repo_name": "server-daily",
        });
        assert_eq!(
            build_email_subject(&p),
            "Backup failed: web-server-01 / server-daily"
        );
    }

    #[test]
    fn subject_backup_warning() {
        let p = serde_json::json!({
            "event_type": "backup_warning",
            "hostname": "db-server-01",
            "repo_name": "db-hourly",
        });
        assert_eq!(
            build_email_subject(&p),
            "Backup warning: db-server-01 / db-hourly"
        );
    }

    #[test]
    fn subject_check_failed() {
        let p = serde_json::json!({
            "event_type": "check_failed",
            "hostname": "myhost",
            "repo_name": "myrepo",
        });
        assert_eq!(build_email_subject(&p), "Check failed: myhost / myrepo");
    }

    #[test]
    fn subject_schedule_auto_disabled() {
        let p = serde_json::json!({
            "event_type": "schedule_auto_disabled",
            "hostname": "web-server-01",
        });
        assert_eq!(
            build_email_subject(&p),
            "Schedule auto-disabled: web-server-01"
        );
    }

    #[test]
    fn subject_backup_skipped_agent_offline() {
        let p = serde_json::json!({
            "event_type": "backup_skipped_agent_offline",
            "hostname": "web-server-01",
        });
        assert_eq!(build_email_subject(&p), "Backup skipped: web-server-01");
    }

    #[test]
    fn subject_backup_success_omits_repo_when_absent() {
        let p = serde_json::json!({
            "event_type": "backup_success",
            "hostname": "myhost",
        });
        assert_eq!(build_email_subject(&p), "Backup succeeded: myhost");
    }

    #[test]
    fn subject_no_hostname_omits_details() {
        let p = serde_json::json!({
            "event_type": "backup_failed",
        });
        assert_eq!(build_email_subject(&p), "Backup failed");
    }

    #[test]
    fn subject_empty_event_type_uses_notification() {
        let p = serde_json::json!({});
        assert_eq!(build_email_subject(&p), "Notification");
    }

    #[test]
    fn body_backup_failed_includes_all_fields() {
        let p = serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "web-server-01",
            "repo_name": "server-daily",
            "schedule_name": "Nightly Server Backup",
            "next_run_at": "2026-06-10T10:00:00Z",
            "archive_name": "web-server-01-2026-06-09T10:00:00.000000",
            "status": "failed",
            "timestamp": "2026-06-09T10:00:00Z",
            "duration_secs": 10,
            "error_message": "repository is locked",
            "activity_url": "https://assimilate.example.com/activity?run_id=abc",
        });
        let body = build_email_body(&p);
        assert!(body.contains("Event:       Backup failed"));
        assert!(body.contains("Host:        web-server-01"));
        assert!(body.contains("Repository:  server-daily"));
        assert!(
            body.contains("Schedule:    Nightly Server Backup (next run: 2026-06-10T10:00:00Z)")
        );
        assert!(body.contains("Archive:     web-server-01-2026-06-09T10:00:00.000000"));
        assert!(body.contains("Duration:    10s"));
        assert!(body.contains("Error:\nrepository is locked"));
        assert!(
            body.contains("View activity log: https://assimilate.example.com/activity?run_id=abc")
        );
        assert!(!body.contains("Status:"));
        assert!(!body.contains('{'));
    }

    #[test]
    fn body_includes_size_and_files_when_present() {
        let p = serde_json::json!({
            "event_type": "backup_success",
            "hostname": "myhost",
            "duration_secs": 272,
            "original_size": 10_737_418_240i64,
            "compressed_size": 2_147_483_648i64,
            "deduplicated_size": 524_288_000i64,
            "files_processed": 184_203,
        });
        let body = build_email_body(&p);
        assert!(body.contains("Duration:    4m 32s"));
        assert!(body.contains("Size:        10.0 GiB -> 2.0 GiB compressed (500.0 MiB new)"));
        assert!(body.contains("Files:       184203 processed"));
    }

    #[test]
    fn body_includes_warnings_list() {
        let p = serde_json::json!({
            "event_type": "backup_warning",
            "hostname": "myhost",
            "warnings": ["file changed while reading", "permission denied: /etc/shadow"],
        });
        let body = build_email_body(&p);
        assert!(
            body.contains(
                "Warnings:\n- file changed while reading\n- permission denied: /etc/shadow"
            )
        );
    }

    #[test]
    fn body_omits_schedule_when_absent() {
        let p = serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "web-server-01",
            "repo_name": "server-daily",
            "timestamp": "2026-06-09T10:00:00Z",
        });
        let body = build_email_body(&p);
        assert!(!body.contains("Schedule:"));
    }

    #[test]
    fn body_schedule_without_next_run_at_omits_parenthetical() {
        let p = serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "web-server-01",
            "schedule_name": "Nightly Server Backup",
        });
        let body = build_email_body(&p);
        assert!(body.ends_with("Schedule:    Nightly Server Backup"));
        assert!(!body.contains("(next run"));
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
        assert!(body.contains("Event:       Agent connected"));
        assert!(body.contains("Host:        web-server-01"));
        assert!(!body.contains("Repository:"));
        assert!(!body.contains("Error:"));
    }

    #[test]
    fn body_empty_payload_returns_notification_label() {
        let body = build_email_body(&serde_json::json!({}));
        assert_eq!(body, "Event:       Notification");
    }

    fn test_email_config() -> EmailConfig {
        EmailConfig {
            smtp_host: "smtp.example.com".to_owned(),
            smtp_port: 587,
            smtp_user: "user".to_owned(),
            smtp_password: "pass".to_owned(),
            from_address: "alerts@example.com".to_owned(),
            to_addresses: vec!["ops@example.com".to_owned()],
            security: SmtpSecurity::Starttls,
            use_tls: false,
            title_template: None,
            body_template: None,
        }
    }

    #[test]
    fn resolve_subject_and_body_falls_back_to_fixed_defaults_when_no_template_configured() {
        let config = test_email_config();
        let p = serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "web-server-01",
            "repo_name": "server-daily",
        });
        let (subject, body) = resolve_subject_and_body(&config, &p);
        assert_eq!(subject, build_email_subject(&p));
        assert_eq!(body, build_email_body(&p));
    }

    #[test]
    fn resolve_subject_and_body_uses_channel_template_when_configured() {
        let mut config = test_email_config();
        config.title_template = Some("{{event}} on {{host}}".to_owned());
        config.body_template = Some("{{dedup_size}} new".to_owned());
        let p = serde_json::json!({
            "event_type": "backup_success",
            "hostname": "web-server-01",
            "deduplicated_size": 524_288_000i64,
        });
        let (subject, body) = resolve_subject_and_body(&config, &p);
        assert_eq!(subject, "Backup succeeded on web-server-01");
        assert_eq!(body, "500.0 MiB new");
    }
}
