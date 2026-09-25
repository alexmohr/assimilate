// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::fmt;

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
    message::{Mailbox, MessageBuilder, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use serde::Deserialize;
use shared::crypto::CryptoError;

use super::{
    NotificationError,
    template::{TemplateFields, format_bytes, format_duration_secs, render_template},
};

/// An email channel's SMTP password as stored in
/// `notification_channels.smtp_password_encrypted`: AES-256-GCM `nonce || ciphertext` under
/// the server's encryption key, the same scheme as a repository passphrase. Only this module
/// decrypts it, right before it logs in to the SMTP server.
#[derive(Clone, PartialEq, Eq)]
pub struct EncryptedSmtpPassword(Vec<u8>);

impl fmt::Debug for EncryptedSmtpPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EncryptedSmtpPassword([REDACTED])")
    }
}

impl From<Vec<u8>> for EncryptedSmtpPassword {
    fn from(stored: Vec<u8>) -> Self {
        Self(stored)
    }
}

impl EncryptedSmtpPassword {
    /// Encrypts a password an admin just typed, ready to be stored.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::EncryptionFailed`] if AES-256-GCM encryption fails.
    pub fn encrypt(plaintext: &str, key: &[u8; 32]) -> Result<Self, CryptoError> {
        shared::crypto::encrypt_passphrase(plaintext, key).map(Self)
    }

    /// The stored `nonce || ciphertext` bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    fn decrypt(&self, key: &[u8; 32]) -> Result<String, NotificationError> {
        Ok(shared::crypto::decrypt_passphrase(&self.0, key)?)
    }
}

/// Where the password for an SMTP login comes from.
#[derive(Debug, Clone, Copy)]
pub enum SmtpPassword<'a> {
    /// The channel has no stored password; it logs in with an empty one, as a channel saved
    /// with a blank password field always has.
    None,
    /// Typed into the dialog for this one check and not stored anywhere.
    Entered(&'a EnteredSmtpPassword),
    /// A saved channel's stored password.
    Stored(&'a EncryptedSmtpPassword),
}

/// A plaintext SMTP password straight from a request body, kept in a type whose `Debug`
/// cannot print it.
#[derive(Clone, Default, Deserialize)]
#[serde(transparent)]
pub struct EnteredSmtpPassword(String);

impl fmt::Debug for EnteredSmtpPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EnteredSmtpPassword([REDACTED])")
    }
}

impl EnteredSmtpPassword {
    /// Wraps a password taken out of a request body.
    #[must_use]
    pub fn new(plaintext: String) -> Self {
        Self(plaintext)
    }

    /// Whether the field was left blank, which on a saved channel means "keep the stored one".
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Encrypts this password for storage.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::EncryptionFailed`] if AES-256-GCM encryption fails.
    pub fn encrypt(&self, key: &[u8; 32]) -> Result<EncryptedSmtpPassword, CryptoError> {
        EncryptedSmtpPassword::encrypt(&self.0, key)
    }
}

impl SmtpPassword<'_> {
    fn reveal(self, key: &[u8; 32]) -> Result<String, NotificationError> {
        match self {
            SmtpPassword::None => Ok(String::new()),
            SmtpPassword::Entered(entered) => Ok(entered.0.clone()),
            SmtpPassword::Stored(stored) => stored.decrypt(key),
        }
    }
}

impl<'a> From<Option<&'a EncryptedSmtpPassword>> for SmtpPassword<'a> {
    fn from(stored: Option<&'a EncryptedSmtpPassword>) -> Self {
        stored.map_or(SmtpPassword::None, SmtpPassword::Stored)
    }
}

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

/// Configuration for an SMTP email notification channel, as stored in
/// `notification_channels.config`. The password is not part of it: it is stored encrypted
/// in its own column (see [`EncryptedSmtpPassword`]) and never enters this JSON.
#[derive(Debug, Deserialize)]
pub struct EmailConfig {
    /// SMTP server hostname.
    pub smtp_host: String,
    /// SMTP server port.
    pub smtp_port: u16,
    /// SMTP authentication username.
    pub smtp_user: String,
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
    (sanitize_header_value(&subject), body)
}

/// Strips CR/LF from a string before it's used as an SMTP header value. Unlike the old fixed
/// `build_email_subject` (which only ever combined `event_label`/`hostname`/`repo_name`, none
/// of which plausibly contain newlines), a channel's own `title_template` can substitute in
/// `{{warnings}}` or `{{error}}` -- both offered as clickable placeholder chips in the editor
/// -- and both can legitimately contain embedded newlines from ordinary agent/borg output, with
/// no adversarial input required. Passing that straight into the `Subject` header risked a
/// garbled multi-line subject at best and SMTP header injection at worst.
fn sanitize_header_value(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

/// Sends `payload` to every recipient, logging in with `password` decrypted under `key`.
///
/// # Errors
///
/// Returns [`NotificationError::Config`] if the notification channel is misconfigured, or
/// [`NotificationError::Crypto`] if the stored password cannot be decrypted.
pub async fn send(
    config: &EmailConfig,
    password: SmtpPassword<'_>,
    key: &[u8; 32],
    payload: &serde_json::Value,
) -> Result<(), NotificationError> {
    let from: Mailbox = config
        .from_address
        .parse()
        .map_err(|e| NotificationError::Config(format!("invalid from address: {e}")))?;

    let (subject, body) = resolve_subject_and_body(config, payload);

    let creds = Credentials::new(config.smtp_user.clone(), password.reveal(key)?);

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

/// Where [`validate_credentials`] should log in, and as whom.
#[derive(Debug, Clone, Copy)]
pub struct SmtpLogin<'a> {
    /// SMTP server hostname.
    pub host: &'a str,
    /// SMTP server port.
    pub port: u16,
    /// SMTP authentication username.
    pub user: &'a str,
    /// SMTP authentication password.
    pub password: SmtpPassword<'a>,
    /// Transport security mode.
    pub security: SmtpSecurity,
}

/// Connects and logs in without sending anything, decrypting a stored password under `key`.
///
/// # Errors
///
/// Returns [`NotificationError::Config`] if the login fails, or [`NotificationError::Crypto`]
/// if the stored password cannot be decrypted.
pub async fn validate_credentials(
    login: SmtpLogin<'_>,
    key: &[u8; 32],
) -> Result<(), NotificationError> {
    let SmtpLogin {
        host,
        port,
        user,
        password,
        security,
    } = login;
    let creds = Credentials::new(user.to_owned(), password.reveal(key)?);
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
    let fields = TemplateFields::from_payload(payload);
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
    fn subject_backup_skipped_repo_offline() {
        let p = serde_json::json!({
            "event_type": "backup_skipped_repo_offline",
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

    #[test]
    fn resolve_subject_and_body_strips_newlines_a_custom_title_template_could_inject() {
        // `warnings` and `error` are ordinary payload fields (not adversarial input) that can
        // legitimately contain embedded newlines, and both are offered as clickable chips for
        // the title field in the editor. Without sanitization, a hostname or a warnings/error
        // placeholder in `title_template` could inject extra lines -- or, depending on the SMTP
        // stack, extra headers -- into the raw `Subject:` header.
        let mut config = test_email_config();
        config.title_template = Some("{{event}}: {{error}}".to_owned());
        let p = serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "web-server-01",
            "error_message": "line one\r\nBcc: attacker@evil.example\nline two",
        });
        let (subject, _) = resolve_subject_and_body(&config, &p);
        assert!(!subject.contains('\r') && !subject.contains('\n'));
        assert_eq!(
            subject,
            "Backup failed: line one  Bcc: attacker@evil.example line two"
        );
    }

    #[test]
    fn deliver_to_channel_backfill_matches_what_the_editor_shows_not_the_legacy_default() {
        // Mirrors what `deliver_to_channel` does before deserializing into `EmailConfig`: a
        // channel that predates the per-channel template feature has no `title_template` in
        // its raw config, so without the backfill it would fall through to the legacy
        // `build_email_subject` below -- a different subject than the one this channel's own
        // "Edit content" panel shows (which always renders `DEFAULT_TITLE_TEMPLATE`).
        let mut raw_config = serde_json::json!({
            "smtp_host": "smtp.example.com",
            "smtp_port": 587,
            "smtp_user": "user",
            "from_address": "alerts@example.com",
            "to_addresses": ["ops@example.com"],
            "security": "starttls",
        });
        super::super::template::apply_default_template(
            &mut raw_config,
            super::super::ChannelType::Email,
        );
        let config: EmailConfig = serde_json::from_value(raw_config).unwrap();

        let p = serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "web-server-01",
            "repo_name": "server-daily",
        });
        let (subject, _) = resolve_subject_and_body(&config, &p);
        assert_eq!(subject, "Backup failed: web-server-01");
        assert_ne!(
            subject,
            build_email_subject(&p),
            "the backfilled config must use the new default template, not the legacy \
             repository-including subject"
        );
    }

    mod smtp_login {
        //! Drives the real lettre transport against a one-connection fake SMTP server, to prove
        //! a stored password is decrypted and sent at login - and only there.

        use super::*;
        use crate::test_support::fake_smtp_server;

        fn plaintext_config(port: u16) -> EmailConfig {
            EmailConfig {
                smtp_host: "127.0.0.1".to_owned(),
                smtp_port: port,
                smtp_user: "alerts".to_owned(),
                security: SmtpSecurity::None,
                ..test_email_config()
            }
        }

        fn key() -> [u8; 32] {
            shared::crypto::derive_key(b"email-send-test-key").unwrap()
        }

        #[tokio::test]
        async fn send_logs_in_with_the_decrypted_stored_password() {
            let (port, server) = fake_smtp_server().await;
            let stored = EncryptedSmtpPassword::encrypt("hunter2", &key()).unwrap();

            send(
                &plaintext_config(port),
                SmtpPassword::Stored(&stored),
                &key(),
                &serde_json::json!({ "event_type": "backup_success", "hostname": "h" }),
            )
            .await
            .unwrap();

            assert_eq!(server.await.unwrap().as_deref(), Some("alerts\0hunter2"));
        }

        #[tokio::test]
        async fn validate_credentials_logs_in_with_the_decrypted_stored_password() {
            let (port, server) = fake_smtp_server().await;
            let stored = EncryptedSmtpPassword::encrypt("hunter2", &key()).unwrap();

            validate_credentials(
                SmtpLogin {
                    host: "127.0.0.1",
                    port,
                    user: "alerts",
                    password: SmtpPassword::Stored(&stored),
                    security: SmtpSecurity::None,
                },
                &key(),
            )
            .await
            .unwrap();

            assert_eq!(server.await.unwrap().as_deref(), Some("alerts\0hunter2"));
        }

        #[tokio::test]
        async fn a_password_encrypted_under_another_key_fails_before_connecting() {
            let other_key = shared::crypto::derive_key(b"some-other-server").unwrap();
            let stored = EncryptedSmtpPassword::encrypt("hunter2", &other_key).unwrap();

            let result = send(
                &plaintext_config(1),
                SmtpPassword::Stored(&stored),
                &key(),
                &serde_json::json!({}),
            )
            .await;

            assert!(matches!(result, Err(NotificationError::Crypto(_))));
        }

        #[test]
        fn stored_password_debug_output_is_redacted() {
            let stored = EncryptedSmtpPassword::encrypt("hunter2", &key()).unwrap();
            assert_eq!(format!("{stored:?}"), "EncryptedSmtpPassword([REDACTED])");
        }
    }
}
