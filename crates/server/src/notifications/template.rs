// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The per-channel `{{placeholder}}` content template shared by every channel type (email,
//! webhook, web push). Each channel carries its own `title_template`/`body_template` --
//! editing one channel's content never touches another's -- and every new channel starts out
//! pre-filled with [`DEFAULT_TITLE_TEMPLATE`]/[`DEFAULT_BODY_TEMPLATE`], which already
//! includes the deduplicated ("new data") size on a successful backup.

/// The default title every new channel starts with. Deliberately omits `{{repository}}`:
/// four of the nine event types (`agent_connected`, `agent_disconnected`,
/// `schedule_auto_disabled`, `backup_skipped_agent_offline`) carry no repository, and this
/// one static default has to read cleanly for all of them -- unlike the multi-line body,
/// where a blank field just leaves an empty line, an empty `{{repository}}` here would leave
/// a dangling `" / "` in the middle of a single line. A channel that only ever sees
/// repository-bearing events can add `/ {{repository}}` back in themselves.
pub(crate) const DEFAULT_TITLE_TEMPLATE: &str = "{{event}}: {{host}}";

/// The default body every new email/webhook channel starts with. Every line is a single
/// `Label: {{value}}` pair, deliberately with no literal words wrapped around more than one
/// placeholder: unlike the old fixed-format builders (`build_email_body`'s `format_size_line`),
/// this template has no control flow to omit a line when its fields are absent, so a line like
/// `Size:  {{original_size}} -> {{compressed_size}} compressed ({{dedup_size}} new)` would
/// render as the nonsensical `Size:   -> compressed ( new)` for the six event types that carry
/// no size data. Splitting each value onto its own `Label: {{value}}` line keeps every line
/// blank-safe the same way `Schedule:`/`Archive:` already are -- a missing value just leaves an
/// empty line -- while still surfacing `{{dedup_size}}` by default on a successful backup.
pub(crate) const DEFAULT_BODY_TEMPLATE: &str = concat!(
    "Event:       {{event}}\n",
    "Host:        {{host}}\n",
    "Repository:  {{repository}}\n",
    "Schedule:    {{schedule}}\n",
    "Archive:     {{archive}}\n",
    "Duration:    {{duration}}\n",
    "Original:    {{original_size}}\n",
    "Compressed:  {{compressed_size}}\n",
    "Dedup:       {{dedup_size}}\n",
    "Files:       {{files}}\n",
    "Time:        {{time}}\n",
    "\n",
    "Warnings:\n",
    "{{warnings}}\n",
    "\n",
    "Error:\n",
    "{{error}}\n",
    "\n",
    "View activity log: {{activity_url}}",
);

/// The default body a new web-push channel starts with. Unlike the multi-line email/webhook
/// default, this stays a single short line: a push toast is typically clipped to one or two
/// lines by the browser, so the old fixed-format `build_push_body` (repo name plus a truncated
/// error) was deliberately terse, and backfilling the long label-per-line
/// [`DEFAULT_BODY_TEMPLATE`] onto push channels would just get silently cut off. `{{repository}}`
/// and `{{error}}` are blank-safe on their own (an absent one just leaves a short gap), matching
/// `build_push_body`'s repo-then-error priority without needing this template engine to support
/// the conditional branching `build_push_body` used.
pub(crate) const DEFAULT_PUSH_BODY_TEMPLATE: &str = "{{repository}} {{error}}";

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

    substitute_placeholders(template, &values)
}

/// Substitutes `{{key}}` tokens in a single left-to-right pass over `template`, looking each
/// key up in `values` and leaving an unrecognized token verbatim. Deliberately not a fold of
/// per-key `String::replace` calls: each of those would rescan the *already-substituted*
/// output for the next key, so a value that happens to contain literal `{{other_key}}` text
/// (e.g. a hostname of `{{error}}`) would get expanded a second time on a later pass even
/// though it never appeared in the template the caller wrote. Scanning the original template
/// only once avoids that.
fn substitute_placeholders(template: &str, values: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after_open = &rest[start.saturating_add(2)..];
        let Some(end) = after_open.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let key = &after_open[..end];
        if let Some((_, value)) = values.iter().find(|(k, _)| *k == key) {
            out.push_str(value);
        } else {
            out.push_str("{{");
            out.push_str(key);
            out.push_str("}}");
        }
        rest = &after_open[end.saturating_add(2)..];
    }
    out.push_str(rest);
    out
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
