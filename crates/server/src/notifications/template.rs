// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The per-channel `{{placeholder}}` content template shared by every channel type (email,
//! webhook, web push). Each channel carries its own `title_template`/`body_template` --
//! editing one channel's content never touches another's -- and every new channel starts out
//! pre-filled with [`DEFAULT_TITLE_TEMPLATE`]/[`DEFAULT_BODY_TEMPLATE`], which already
//! includes the deduplicated ("new data") size on a successful backup.

/// The default title every new channel starts with.
pub(crate) const DEFAULT_TITLE_TEMPLATE: &str = "{{event}}: {{host}} / {{repository}}";

/// The default body every new channel starts with -- deliberately includes
/// `{{dedup_size}}` on the `Size:` line so the deduplicated size shows up on a successful
/// backup notification without the user having to add it themselves.
pub(crate) const DEFAULT_BODY_TEMPLATE: &str =
    "Event:       {{event}}\nHost:        {{host}}\nRepository:  {{repository}}\nSchedule:    \
     {{schedule}}\nArchive:     {{archive}}\nDuration:    {{duration}}\nSize:        \
     {{original_size}} -> {{compressed_size}} compressed ({{dedup_size}} new)\nFiles:       \
     {{files}} processed\nTime:        \
     {{time}}\n\nWarnings:\n{{warnings}}\n\nError:\n{{error}}\n\nView activity log: \
     {{activity_url}}";

/// Fills in `title_template`/`body_template` on a channel config with the shared defaults
/// when the caller didn't supply them, so every channel -- created through the UI or the raw
/// API alike -- has an explicit, persisted content template from the moment it exists rather
/// than relying on a client to have pre-filled a form. A no-op once both keys are present
/// (e.g. an update that already carries the channel's current template).
pub(crate) fn apply_default_template(config: &mut serde_json::Value) {
    let Some(obj) = config.as_object_mut() else {
        return;
    };
    obj.entry("title_template")
        .or_insert_with(|| serde_json::Value::String(DEFAULT_TITLE_TEMPLATE.to_owned()));
    obj.entry("body_template")
        .or_insert_with(|| serde_json::Value::String(DEFAULT_BODY_TEMPLATE.to_owned()));
}

/// Renders a byte count as a human-readable size (e.g. `12.4 GiB`). Sizes in a payload are
/// carried as `i64` (JSON has no unsigned integer type), so this adapts to the `u64` shared
/// formatter rather than duplicating it -- see `shared::format::format_bytes`.
pub(crate) fn format_bytes(bytes: i64) -> String {
    shared::format::format_bytes(u64::try_from(bytes).unwrap_or(0))
}

/// Renders a duration as e.g. `1h 2m 3s`, `4m 5s`, or `6s`.
pub(crate) fn format_duration_secs(secs: i64) -> String {
    let secs = u64::try_from(secs).unwrap_or(0);
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

/// Fields pulled out of a notification payload, gathered up front so both [`render_template`]
/// and the fixed-format email builders (`super::email::build_email_body`, which needs every
/// field here except `status`) read as a flat list rather than being interleaved with
/// `payload.get(...)` boilerplate. Shared rather than duplicated per caller -- the two field
/// sets were identical but for that one field.
pub(crate) struct TemplateFields<'a> {
    pub(crate) event_type: &'a str,
    pub(crate) hostname: &'a str,
    pub(crate) repo_name: &'a str,
    pub(crate) status: &'a str,
    pub(crate) schedule_name: Option<&'a str>,
    pub(crate) next_run_at: Option<&'a str>,
    pub(crate) timestamp: &'a str,
    pub(crate) error_message: Option<&'a str>,
    pub(crate) archive_name: Option<&'a str>,
    pub(crate) duration_secs: Option<i64>,
    pub(crate) original_size: Option<i64>,
    pub(crate) compressed_size: Option<i64>,
    pub(crate) deduplicated_size: Option<i64>,
    pub(crate) files_processed: Option<i64>,
    pub(crate) warnings: Vec<&'a str>,
    pub(crate) activity_url: Option<&'a str>,
}

impl<'a> TemplateFields<'a> {
    pub(crate) fn from_payload(payload: &'a serde_json::Value) -> Self {
        let str_field = |key: &str| payload.get(key).and_then(serde_json::Value::as_str);
        let int_field = |key: &str| payload.get(key).and_then(serde_json::Value::as_i64);
        Self {
            event_type: str_field("event_type").unwrap_or(""),
            hostname: str_field("hostname").unwrap_or(""),
            repo_name: str_field("repo_name").unwrap_or(""),
            status: str_field("status").unwrap_or(""),
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

/// Renders a user-authored title/body template by substituting `{{placeholder}}` tokens --
/// `event`, `host`, `repository`, `status`, `schedule`, `next_run`, `archive`, `duration`,
/// `original_size`, `compressed_size`, `dedup_size`, `files`, `time`, `warnings`, `error`,
/// `activity_url` (the same list `frontend/src/utils/notificationTemplate.ts` offers as
/// clickable chips) -- with values pulled from the notification payload. A placeholder for a
/// field the event doesn't carry (e.g. `{{dedup_size}}` on a `check_failed` event)
/// substitutes an empty string rather than dropping the surrounding text -- the template is
/// free-form text the user laid out themselves, so there's no line structure to conditionally
/// remove. An unrecognized `{{...}}` token is left verbatim, so a typo in the template is
/// visible in the delivered notification instead of silently disappearing.
pub(crate) fn render_template(template: &str, payload: &serde_json::Value) -> String {
    let fields = TemplateFields::from_payload(payload);
    let event_label = super::event_label(fields.event_type);

    let warnings = if fields.warnings.is_empty() {
        String::new()
    } else {
        fields
            .warnings
            .iter()
            .map(|w| format!("- {w}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let values: [(&str, String); 16] = [
        ("event", event_label.to_owned()),
        ("host", fields.hostname.to_owned()),
        ("repository", fields.repo_name.to_owned()),
        ("status", fields.status.to_owned()),
        (
            "schedule",
            fields.schedule_name.unwrap_or_default().to_owned(),
        ),
        (
            "next_run",
            fields.next_run_at.unwrap_or_default().to_owned(),
        ),
        (
            "archive",
            fields.archive_name.unwrap_or_default().to_owned(),
        ),
        (
            "duration",
            fields
                .duration_secs
                .map_or_else(String::new, format_duration_secs),
        ),
        (
            "original_size",
            fields.original_size.map_or_else(String::new, format_bytes),
        ),
        (
            "compressed_size",
            fields
                .compressed_size
                .map_or_else(String::new, format_bytes),
        ),
        (
            "dedup_size",
            fields
                .deduplicated_size
                .map_or_else(String::new, format_bytes),
        ),
        (
            "files",
            fields
                .files_processed
                .map_or_else(String::new, |n| n.to_string()),
        ),
        ("time", fields.timestamp.to_owned()),
        ("warnings", warnings),
        ("error", fields.error_message.unwrap_or_default().to_owned()),
        (
            "activity_url",
            fields.activity_url.unwrap_or_default().to_owned(),
        ),
    ];

    values
        .into_iter()
        .fold(template.to_owned(), |out, (key, value)| {
            out.replace(&format!("{{{{{key}}}}}"), &value)
        })
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
            rendered.contains("Size:        10.0 GiB -> 2.0 GiB compressed (500.0 MiB new)"),
            "the default template must surface the deduplicated size on a successful backup \
             without the user having to add it: {rendered}"
        );
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
        apply_default_template(&mut config);
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
    fn apply_default_template_never_overwrites_an_existing_template() {
        let mut config = serde_json::json!({
            "url": "https://hooks.example.com",
            "title_template": "custom title",
            "body_template": "custom body",
        });
        apply_default_template(&mut config);
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
    fn default_title_template_renders_readable_title() {
        let p = serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "web-server-01",
            "repo_name": "daily-backup",
        });
        assert_eq!(
            render_template(DEFAULT_TITLE_TEMPLATE, &p),
            "Backup failed: web-server-01 / daily-backup"
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
