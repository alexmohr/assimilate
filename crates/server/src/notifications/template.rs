// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The per-channel `{{placeholder}}` content template shared by every channel type (email,
//! webhook, web push). Each channel carries its own `title_template`/`body_template` --
//! editing one channel's content never touches another's -- and every new channel starts out
//! pre-filled with [`DEFAULT_TITLE_TEMPLATE`]/[`DEFAULT_BODY_TEMPLATE`], which already
//! includes the deduplicated ("new data") size on a successful backup.

pub(crate) use domain::notification::template::{
    DEFAULT_BODY_TEMPLATE, DEFAULT_PUSH_BODY_TEMPLATE, DEFAULT_TITLE_TEMPLATE, TemplateFields,
    format_bytes, format_duration_secs, render_template,
};

/// Fills in `title_template`/`body_template` on a channel config with the shared defaults
/// when the caller didn't supply them, so every channel -- created through the UI or the raw
/// API alike -- has an explicit, persisted content template from the moment it exists rather
/// than relying on a client to have pre-filled a form. A no-op once both keys are present
/// (e.g. an update that already carries the channel's current template). `channel_type` picks
/// the body default: web push gets the short [`DEFAULT_PUSH_BODY_TEMPLATE`] instead of the
/// multi-line [`DEFAULT_BODY_TEMPLATE`], since a push toast has no room for the latter.
pub(crate) fn apply_default_template(
    config: &mut serde_json::Value,
    channel_type: super::ChannelType,
) {
    let Some(obj) = config.as_object_mut() else {
        return;
    };
    let default_body = if channel_type == super::ChannelType::WebPush {
        DEFAULT_PUSH_BODY_TEMPLATE
    } else {
        DEFAULT_BODY_TEMPLATE
    };
    obj.entry("title_template")
        .or_insert_with(|| serde_json::Value::String(DEFAULT_TITLE_TEMPLATE.to_owned()));
    obj.entry("body_template")
        .or_insert_with(|| serde_json::Value::String(default_body.to_owned()));
}

/// Rejects a `title_template`/`body_template` that's present but blank (empty or
/// whitespace-only). `Some("")` is a different value than `None` to every delivery path
/// (`resolve_subject_and_body`, `push_title_and_body`, `build_payload`), which all treat
/// "template present" -- even if blank -- as "don't fall back to the built-in default", so a
/// blank template silently sends an empty title/body instead of erroring or falling back.
/// Checked directly against the raw config `Value` so it applies uniformly across all three
/// channel types without needing a shared config trait. Returns the name of whichever field
/// is blank.
pub(crate) fn validate_template_fields(config: &serde_json::Value) -> Result<(), &'static str> {
    let Some(obj) = config.as_object() else {
        return Ok(());
    };
    for field in ["title_template", "body_template"] {
        if obj
            .get(field)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|s| s.trim().is_empty())
        {
            return Err(field);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_template_substitutes_every_placeholder() {
        let p = serde_json::json!({
            "event_type": "backup_success",
            "hostname": "web-server-01",
            "repo_name": "daily-backup",
            "status": "success",
            "schedule_name": "Nightly",
            "next_run_at": "2026-06-10T10:00:00Z",
            "archive_name": "web-server-01-2026-06-09",
            "duration_secs": 272,
            "original_size": 10_737_418_240i64,
            "compressed_size": 2_147_483_648i64,
            "deduplicated_size": 524_288_000i64,
            "files_processed": 184_203,
            "timestamp": "2026-06-09T10:00:00Z",
            "warnings": ["disk almost full"],
            "error_message": "none",
            "activity_url": "https://assimilate.example.com/activity?run_id=abc",
        });
        let rendered = render_template(
            "{{event}} {{host}} {{repository}} {{status}} {{schedule}} {{next_run}} {{archive}} \
             {{duration}} {{original_size}} {{compressed_size}} {{dedup_size}} {{files}} {{time}} \
             {{warnings}} {{error}} {{activity_url}}",
            &p,
        );
        assert_eq!(
            rendered,
            "Backup succeeded web-server-01 daily-backup success Nightly \
             2026-06-10T10:00:00Z web-server-01-2026-06-09 4m 32s 10.0 GiB 2.0 GiB \
             500.0 MiB 184203 2026-06-09T10:00:00Z - disk almost full none \
             https://assimilate.example.com/activity?run_id=abc"
        );
    }

    #[test]
    fn default_body_template_shows_dedup_size_on_a_successful_backup() {
        let p = serde_json::json!({
            "event_type": "backup_success",
            "hostname": "web-server-01",
            "repo_name": "daily-backup",
            "original_size": 10_737_418_240i64,
            "compressed_size": 2_147_483_648i64,
            "deduplicated_size": 524_288_000i64,
        });
        let rendered = render_template(DEFAULT_BODY_TEMPLATE, &p);
        assert!(
            rendered.contains("Dedup:       500.0 MiB"),
            "the default template must surface the deduplicated size on a successful backup \
             without the user having to add it: {rendered}"
        );
    }

    #[test]
    fn default_body_template_leaves_size_and_file_lines_blank_for_a_sizeless_event() {
        for event_type in [
            "agent_connected",
            "agent_disconnected",
            "schedule_auto_disabled",
            "backup_skipped_agent_offline",
            "check_success",
            "check_failed",
        ] {
            let p = serde_json::json!({ "event_type": event_type, "hostname": "web-server-01" });
            let rendered = render_template(DEFAULT_BODY_TEMPLATE, &p);
            for label in ["Original:", "Compressed:", "Dedup:", "Files:"] {
                let line = rendered
                    .lines()
                    .find(|l| l.starts_with(label))
                    .unwrap_or_else(|| panic!("{event_type} is missing the {label} line"));
                assert_eq!(
                    line.trim(),
                    label,
                    "{event_type} rendered a non-blank {label} line with no data: {line:?}"
                );
            }
        }
    }

    #[test]
    fn render_template_missing_field_substitutes_empty_string() {
        let p = serde_json::json!({ "event_type": "agent_connected", "hostname": "myhost" });
        let rendered = render_template("dedup=[{{dedup_size}}] err=[{{error}}]", &p);
        assert_eq!(rendered, "dedup=[] err=[]");
    }

    #[test]
    fn render_template_leaves_unknown_placeholder_verbatim() {
        let p = serde_json::json!({ "event_type": "agent_connected" });
        let rendered = render_template("{{not_a_real_field}}", &p);
        assert_eq!(rendered, "{{not_a_real_field}}");
    }

    #[test]
    fn all_template_placeholders_appear_in_default_body_template() {
        let placeholders = [
            "event",
            "host",
            "repository",
            "schedule",
            "archive",
            "duration",
            "original_size",
            "compressed_size",
            "dedup_size",
            "files",
            "time",
            "warnings",
            "error",
            "activity_url",
        ];
        for placeholder in placeholders {
            assert!(
                DEFAULT_BODY_TEMPLATE.contains(&format!("{{{{{placeholder}}}}}")),
                "DEFAULT_BODY_TEMPLATE is missing {{{{{placeholder}}}}}"
            );
        }
    }

    #[test]
    fn apply_default_template_fills_in_missing_fields() {
        let mut config = serde_json::json!({ "url": "https://hooks.example.com" });
        apply_default_template(&mut config, super::super::ChannelType::Webhook);
        assert_eq!(
            config
                .get("title_template")
                .and_then(serde_json::Value::as_str),
            Some(DEFAULT_TITLE_TEMPLATE)
        );
        assert_eq!(
            config
                .get("body_template")
                .and_then(serde_json::Value::as_str),
            Some(DEFAULT_BODY_TEMPLATE)
        );
    }

    #[test]
    fn apply_default_template_gives_web_push_the_short_push_body_instead_of_the_email_default() {
        let mut config = serde_json::json!({ "user_id": 1 });
        apply_default_template(&mut config, super::super::ChannelType::WebPush);
        assert_eq!(
            config
                .get("body_template")
                .and_then(serde_json::Value::as_str),
            Some(DEFAULT_PUSH_BODY_TEMPLATE)
        );
        assert_ne!(DEFAULT_PUSH_BODY_TEMPLATE, DEFAULT_BODY_TEMPLATE);
    }

    #[test]
    fn apply_default_template_never_overwrites_an_existing_template() {
        let mut config = serde_json::json!({
            "url": "https://hooks.example.com",
            "title_template": "custom title",
            "body_template": "custom body",
        });
        apply_default_template(&mut config, super::super::ChannelType::Webhook);
        assert_eq!(
            config
                .get("title_template")
                .and_then(serde_json::Value::as_str),
            Some("custom title")
        );
        assert_eq!(
            config
                .get("body_template")
                .and_then(serde_json::Value::as_str),
            Some("custom body")
        );
    }

    #[test]
    fn validate_template_fields_rejects_a_blank_title_or_body() {
        let blank_title = serde_json::json!({ "title_template": "   ", "body_template": "ok" });
        assert_eq!(
            validate_template_fields(&blank_title),
            Err("title_template")
        );

        let blank_body = serde_json::json!({ "title_template": "ok", "body_template": "" });
        assert_eq!(validate_template_fields(&blank_body), Err("body_template"));
    }

    #[test]
    fn validate_template_fields_accepts_absent_or_non_blank_templates() {
        let absent = serde_json::json!({ "url": "https://hooks.example.com" });
        assert_eq!(validate_template_fields(&absent), Ok(()));

        let present = serde_json::json!({ "title_template": "{{event}}", "body_template": "x" });
        assert_eq!(validate_template_fields(&present), Ok(()));
    }

    #[test]
    fn default_title_template_renders_readable_title() {
        let p = serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "web-server-01",
            "repo_name": "daily-backup",
        });
        assert_eq!(
            render_template(DEFAULT_TITLE_TEMPLATE, &p),
            "Backup failed: web-server-01"
        );
    }

    #[test]
    fn default_title_template_never_leaves_a_dangling_separator_for_a_repo_less_event() {
        for event_type in [
            "agent_connected",
            "agent_disconnected",
            "schedule_auto_disabled",
            "backup_skipped_agent_offline",
        ] {
            let p = serde_json::json!({ "event_type": event_type, "hostname": "web-server-01" });
            let rendered = render_template(DEFAULT_TITLE_TEMPLATE, &p);
            assert!(
                !rendered.ends_with('/') && !rendered.ends_with("/ "),
                "{event_type} produced a dangling separator: {rendered:?}"
            );
        }
    }

    #[test]
    fn render_template_does_not_re_expand_a_substituted_values_own_placeholder_syntax() {
        let p = serde_json::json!({
            "event_type": "backup_success",
            "hostname": "{{error}}",
            "error_message": "should not leak into host",
        });
        let rendered = render_template("host=[{{host}}] error=[{{error}}]", &p);
        assert_eq!(
            rendered,
            "host=[{{error}}] error=[should not leak into host]"
        );
    }

    #[test]
    fn format_bytes_renders_expected_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(13_314_562_048), "12.4 GiB");
    }

    #[test]
    fn format_duration_secs_renders_expected_units() {
        assert_eq!(format_duration_secs(9), "9s");
        assert_eq!(format_duration_secs(272), "4m 32s");
        assert_eq!(format_duration_secs(3725), "1h 2m 5s");
    }
}
