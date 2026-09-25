// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/// The default title every new channel starts with. Deliberately omits `{{repository}}`:
/// four of the nine event types (`agent_connected`, `agent_disconnected`,
/// `schedule_auto_disabled`, `backup_skipped_agent_offline`) carry no repository, and this
/// one static default has to read cleanly for all of them -- unlike the multi-line body,
/// where a blank field just leaves an empty line, an empty `{{repository}}` here would leave
/// a dangling `" / "` in the middle of a single line. A channel that only ever sees
/// repository-bearing events can add `/ {{repository}}` back in themselves.
pub const DEFAULT_TITLE_TEMPLATE: &str = "{{event}}: {{host}}";

/// The default body every new email/webhook channel starts with. Every line is a single
/// `Label: {{value}}` pair, deliberately with no literal words wrapped around more than one
/// placeholder: unlike the old fixed-format builders (`build_email_body`'s `format_size_line`),
/// this template has no control flow to omit a line when its fields are absent, so a line like
/// `Size:  {{original_size}} -> {{compressed_size}} compressed ({{dedup_size}} new)` would
/// render as the nonsensical `Size:   -> compressed ( new)` for the six event types that carry
/// no size data. Splitting each value onto its own `Label: {{value}}` line keeps every line
/// blank-safe the same way `Schedule:`/`Archive:` already are -- a missing value just leaves an
/// empty line -- while still surfacing `{{dedup_size}}` by default on a successful backup.
pub const DEFAULT_BODY_TEMPLATE: &str = concat!(
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
/// error, falling back to hostname when neither is present) was deliberately terse, and
/// backfilling the long label-per-line [`DEFAULT_BODY_TEMPLATE`] onto push channels would just
/// get silently cut off. Leads with `{{host}}` -- already duplicated between the title and body
/// in the multi-line default (its `Host:` line), so this isn't a new inconsistency -- so an
/// event with neither `{{repository}}` nor `{{error}}` (`agent_connected`/`agent_disconnected`)
/// still renders something instead of a lone blank space, matching `build_push_body`'s
/// hostname fallback without needing this template engine to support the conditional branching
/// `build_push_body` used. Trailing blank-safe fields just leave harmless trailing whitespace.
pub const DEFAULT_PUSH_BODY_TEMPLATE: &str = "{{host}} {{repository}} {{error}}";

/// Renders a byte count as a human-readable size (e.g. `12.4 GiB`). Sizes in a payload are
/// carried as `i64` (JSON has no unsigned integer type), so this adapts to the `u64` shared
/// formatter rather than duplicating it -- see `crate::format::format_bytes`.
#[must_use]
pub fn format_bytes(bytes: i64) -> String {
    crate::format::format_bytes(u64::try_from(bytes).unwrap_or(0))
}

/// Renders a duration as e.g. `1h 2m 3s`, `4m 5s`, or `6s`.
#[must_use]
pub fn format_duration_secs(secs: i64) -> String {
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
/// and the fixed-format email builders (the server's `build_email_body`, which needs every
/// field here except `status`) read as a flat list rather than being interleaved with
/// `payload.get(...)` boilerplate. Shared rather than duplicated per caller -- the two field
/// sets were identical but for that one field.
pub struct TemplateFields<'a> {
    /// The raw `event_type` string; see [`super::EventType`].
    pub event_type: &'a str,
    /// The agent's hostname.
    pub hostname: &'a str,
    /// The repository name, empty for events without one.
    pub repo_name: &'a str,
    /// The raw backup/check status string.
    pub status: &'a str,
    /// The schedule that triggered the event, if any.
    pub schedule_name: Option<&'a str>,
    /// When the schedule runs next, if known.
    pub next_run_at: Option<&'a str>,
    /// When the event happened.
    pub timestamp: &'a str,
    /// The error message of a failed run, if any.
    pub error_message: Option<&'a str>,
    /// The borg archive the run created, if any.
    pub archive_name: Option<&'a str>,
    /// How long the run took, in seconds.
    pub duration_secs: Option<i64>,
    /// Size of the backed-up data before compression, in bytes.
    pub original_size: Option<i64>,
    /// Size after compression, in bytes.
    pub compressed_size: Option<i64>,
    /// New data written after deduplication, in bytes.
    pub deduplicated_size: Option<i64>,
    /// Number of files the run processed.
    pub files_processed: Option<i64>,
    /// Warnings the run reported.
    pub warnings: Vec<&'a str>,
    /// Link to the run in the Activity Log.
    pub activity_url: Option<&'a str>,
}

impl<'a> TemplateFields<'a> {
    /// Reads every field out of a notification payload, leaving absent ones empty.
    #[must_use]
    pub fn from_payload(payload: &'a serde_json::Value) -> Self {
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
#[must_use]
pub fn render_template(template: &str, payload: &serde_json::Value) -> String {
    substitute_placeholders(template, &placeholder_values(payload))
}

/// Every placeholder key [`render_template`] substitutes, in the order the
/// frontend lists them. Derived from the substitution table itself so the two
/// can't drift apart.
pub fn placeholder_keys() -> impl Iterator<Item = &'static str> {
    placeholder_values(&serde_json::Value::Null)
        .into_iter()
        .map(|(key, _)| key)
}

fn placeholder_values(payload: &serde_json::Value) -> [(&'static str, String); 16] {
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

    [
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
    ]
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
    use super::{DEFAULT_BODY_TEMPLATE, placeholder_keys};

    #[test]
    fn placeholder_keys_lists_every_substituted_key_once() {
        let keys: Vec<_> = placeholder_keys().collect();
        assert_eq!(keys.len(), 16);
        assert_eq!(keys.first(), Some(&"event"));
        assert_eq!(keys.last(), Some(&"activity_url"));
        let mut unique = keys.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), keys.len());
    }

    #[test]
    fn default_body_only_uses_known_placeholders() {
        let keys: Vec<_> = placeholder_keys().collect();
        let used = DEFAULT_BODY_TEMPLATE
            .split("{{")
            .skip(1)
            .filter_map(|rest| rest.split_once("}}").map(|(key, _)| key));
        used.for_each(|key| assert!(keys.contains(&key), "unknown placeholder {key}"));
    }
}
