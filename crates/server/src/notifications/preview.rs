// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Renders a channel's content template against a sample event for the
//! notification editor's live preview, with the same [`render_template`] a
//! channel delivers with, so the preview can never differ from what is sent.

use serde_json::{Value, json};
use shared::notifications::{EventType, TemplatePreviewRequest, TemplatePreviewResponse};

use super::template::render_template;

/// Renders both templates of `request` against the sample for its event.
pub(crate) fn render_preview(request: &TemplatePreviewRequest) -> TemplatePreviewResponse {
    let payload = sample_payload(request.event_type);
    TemplatePreviewResponse {
        title: render_template(&request.title_template, &payload),
        body: render_template(&request.body_template, &payload),
    }
}

/// A realistic payload for `event_type`, carrying the fields a real event of
/// that type carries and none it doesn't, so blank placeholders preview blank.
fn sample_payload(event_type: EventType) -> Value {
    match event_type {
        EventType::BackupSuccess => backup_success_sample(),
        EventType::BackupWarning => backup_warning_sample(),
        EventType::BackupFailed => json!({
            "event_type": "backup_failed",
            "hostname": "db-server-02",
            "repo_name": "db-hourly",
            "status": "failed",
            "schedule_name": "Hourly DB Backup",
            "duration_secs": 8,
            "timestamp": "2026-09-17T14:00:08Z",
            "error_message": "repository is locked by another process",
        }),
        EventType::CheckSuccess => json!({
            "event_type": "check_success",
            "hostname": "web-server-01",
            "repo_name": "daily-backup",
            "status": "success",
            "timestamp": "2026-09-17T04:00:00Z",
        }),
        EventType::CheckFailed => json!({
            "event_type": "check_failed",
            "hostname": "web-server-01",
            "repo_name": "daily-backup",
            "status": "failed",
            "timestamp": "2026-09-17T04:00:00Z",
            "error_message": "integrity check failed",
        }),
        EventType::AgentConnected => json!({
            "event_type": "agent_connected",
            "hostname": "web-server-01",
            "timestamp": "2026-09-17T07:58:03Z",
        }),
        EventType::AgentDisconnected => json!({
            "event_type": "agent_disconnected",
            "hostname": "web-server-01",
            "timestamp": "2026-09-17T07:58:03Z",
        }),
        EventType::ScheduleAutoDisabled => json!({
            "event_type": "schedule_auto_disabled",
            "hostname": "web-server-01",
            "timestamp": "2026-09-17T07:58:03Z",
            "error_message": "agent 'web-server-01' stayed unreachable",
        }),
        EventType::BackupSkippedAgentOffline => json!({
            "event_type": "backup_skipped_agent_offline",
            "hostname": "web-server-01",
            "timestamp": "2026-09-17T07:58:03Z",
            "error_message": "agent 'web-server-01' is offline",
        }),
        // Carries a repo_name where its agent-offline sibling cannot: the agent
        // is connected here, and the repository is the thing that is not there.
        EventType::BackupSkippedRepoOffline => json!({
            "event_type": "backup_skipped_repo_offline",
            "hostname": "db-server-02",
            "repo_name": "db-hourly",
            "schedule_name": "Hourly DB Backup",
            "timestamp": "2026-09-17T08:00:04Z",
            "error_message": "the host for repository 'db-hourly' did not answer SSH",
        }),
        EventType::BackupFileChanged => backup_file_changed_sample(),
        EventType::BackupCatchUpAbandoned => json!({
            "event_type": "backup_catch_up_abandoned",
            "hostname": "laptop-01",
            "repo_name": "daily-backup",
            "status": "abandoned",
            "schedule_name": "Nightly Server Backup",
            "timestamp": "2026-09-18T03:00:00Z",
            "error_message": "host 'laptop-01' did not come back within 1 day",
        }),
    }
}

fn backup_success_sample() -> Value {
    json!({
        "event_type": "backup_success",
        "hostname": "web-server-01",
        "repo_name": "daily-backup",
        "status": "success",
        "schedule_name": "Nightly Server Backup",
        "next_run_at": "2026-09-18T03:00:00Z",
        "archive_name": "web-server-01-2026-09-17T03:00:00",
        "duration_secs": 272,
        "original_size": 10_737_418_240i64,
        "compressed_size": 2_147_483_648i64,
        "deduplicated_size": 524_288_000,
        "files_processed": 184_203,
        "timestamp": "2026-09-17T03:04:32Z",
        "warnings": [],
        "activity_url": "https://backups.example.com/activity?category=backup&run_id=8f2e1a3c",
    })
}

fn backup_warning_sample() -> Value {
    json!({
        "event_type": "backup_warning",
        "hostname": "web-server-01",
        "repo_name": "daily-backup",
        "status": "warning",
        "schedule_name": "Nightly Server Backup",
        "duration_secs": 301,
        "original_size": 10_800_000_000i64,
        "compressed_size": 2_200_000_000i64,
        "deduplicated_size": 610_000_000,
        "files_processed": 184_310,
        "timestamp": "2026-09-17T03:05:01Z",
        "warnings": ["file changed while reading: /var/log/app.log"],
    })
}

fn backup_file_changed_sample() -> Value {
    json!({
        "event_type": "backup_file_changed",
        "hostname": "web-server-01",
        "repo_name": "daily-backup",
        "status": "warning",
        "schedule_name": "Nightly Server Backup",
        "duration_secs": 298,
        "original_size": 10_790_000_000i64,
        "compressed_size": 2_190_000_000i64,
        "deduplicated_size": 590_000_000,
        "files_processed": 184_288,
        "timestamp": "2026-09-17T03:04:58Z",
        "warnings": ["/var/log/app.log: file changed while we backed it up"],
    })
}

#[cfg(test)]
mod tests {
    use shared::notifications::{EventType, TemplatePreviewRequest};

    use super::{render_preview, sample_payload};
    use crate::notifications::template::{
        DEFAULT_BODY_TEMPLATE, DEFAULT_PUSH_BODY_TEMPLATE, DEFAULT_TITLE_TEMPLATE,
    };

    fn preview(title: &str, body: &str, event_type: EventType) -> (String, String) {
        let rendered = render_preview(&TemplatePreviewRequest {
            title_template: title.to_owned(),
            body_template: body.to_owned(),
            event_type,
        });
        (rendered.title, rendered.body)
    }

    #[test]
    fn default_title_reads_cleanly_for_a_successful_backup() {
        let (title, _) = preview(DEFAULT_TITLE_TEMPLATE, "", EventType::BackupSuccess);
        assert_eq!(title, "Backup succeeded: web-server-01");
    }

    #[test]
    fn default_body_shows_the_deduplicated_size_for_a_successful_backup() {
        let (_, body) = preview("", DEFAULT_BODY_TEMPLATE, EventType::BackupSuccess);
        assert!(body.contains("Dedup:       500.0 MiB"), "{body}");
        assert!(body.contains("Duration:    4m 32s"), "{body}");
    }

    #[test]
    fn a_failed_backup_previews_its_own_host_and_error() {
        let (title, body) = preview(
            DEFAULT_TITLE_TEMPLATE,
            DEFAULT_BODY_TEMPLATE,
            EventType::BackupFailed,
        );
        assert_eq!(title, "Backup failed: db-server-02");
        assert!(
            body.contains("repository is locked by another process"),
            "{body}"
        );
        assert!(body.contains("Dedup:       \n"), "{body}");
    }

    #[test]
    fn push_default_falls_back_to_the_hostname_for_a_repo_less_event() {
        let (_, body) = preview("", DEFAULT_PUSH_BODY_TEMPLATE, EventType::AgentConnected);
        assert_eq!(body.trim(), "web-server-01");
    }

    #[test]
    fn unknown_placeholders_stay_verbatim() {
        let (title, _) = preview("{{not_a_field}}", "", EventType::BackupSuccess);
        assert_eq!(title, "{{not_a_field}}");
    }

    /// The defaults and placeholder keys `frontend/src/utils/notificationTemplate.ts` offers.
    #[derive(serde::Deserialize)]
    struct SharedTemplate {
        default_title_template: String,
        default_body_template: String,
        default_push_body_template: String,
        placeholder_keys: Vec<String>,
    }

    fn shared_template() -> SharedTemplate {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/parity/notification_template.json"
        )))
        .unwrap()
    }

    #[test]
    fn defaults_match_the_ones_the_editor_pre_fills() {
        let shared = shared_template();
        assert_eq!(shared.default_title_template, DEFAULT_TITLE_TEMPLATE);
        assert_eq!(shared.default_body_template, DEFAULT_BODY_TEMPLATE);
        assert_eq!(
            shared.default_push_body_template,
            DEFAULT_PUSH_BODY_TEMPLATE
        );
    }

    #[test]
    fn every_placeholder_the_editor_offers_is_substituted() {
        for key in shared_template().placeholder_keys {
            let token = format!("{{{{{key}}}}}");
            let (title, _) = preview(&token, "", EventType::BackupSuccess);
            assert_ne!(
                title, token,
                "{{{{{key}}}}} is not a placeholder the renderer knows"
            );
        }
    }

    #[test]
    fn every_event_has_a_sample_of_its_own_type() {
        let all = [
            EventType::BackupSuccess,
            EventType::BackupWarning,
            EventType::BackupFailed,
            EventType::CheckSuccess,
            EventType::CheckFailed,
            EventType::AgentConnected,
            EventType::AgentDisconnected,
            EventType::ScheduleAutoDisabled,
            EventType::BackupSkippedAgentOffline,
            EventType::BackupSkippedRepoOffline,
            EventType::BackupFileChanged,
            EventType::BackupCatchUpAbandoned,
        ];
        for event_type in all {
            let payload = sample_payload(event_type);
            assert_eq!(
                payload
                    .get("event_type")
                    .and_then(serde_json::Value::as_str),
                Some(event_type.to_string().as_str()),
            );
        }
    }
}
