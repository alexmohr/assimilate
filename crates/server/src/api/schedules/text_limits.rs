// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Length caps on the free-form strings a schedule create or update carries.

use super::{
    AgentBackupSources, AgentCommands, AgentExcludePatterns, AgentFileChangePatterns,
    AgentIncludePatterns, CreateScheduleRequest, UpdateScheduleRequest,
};
use crate::{
    api::helpers::{self, MaxLen},
    error::ApiError,
};

/// The strings shared by [`CreateScheduleRequest`] and
/// [`UpdateScheduleRequest`], borrowed so both are checked by the same rules
/// before anything is written.
pub(super) struct ScheduleTextFields<'a> {
    name: Option<&'a str>,
    cron_expression: &'a str,
    exclude_patterns_raw: Option<&'a str>,
    include_patterns_raw: Option<&'a str>,
    file_change_patterns_raw: Option<&'a str>,
    backup_sources: Option<&'a [String]>,
    backup_sources_per_agent: Option<&'a [AgentBackupSources]>,
    exclude_patterns_per_agent: Option<&'a [AgentExcludePatterns]>,
    include_patterns_per_agent: Option<&'a [AgentIncludePatterns]>,
    file_change_patterns_per_agent: Option<&'a [AgentFileChangePatterns]>,
    commands_per_agent: Option<&'a [AgentCommands]>,
}

/// Both request types carry these fields under the same names and types, so
/// one conversion body serves both.
macro_rules! schedule_text_fields_from {
    ($($request:ty),+) => {$(
        impl<'a> From<&'a $request> for ScheduleTextFields<'a> {
            fn from(req: &'a $request) -> Self {
                Self {
                    name: req.name.as_deref(),
                    cron_expression: &req.cron_expression,
                    exclude_patterns_raw: req.exclude_patterns_raw.as_deref(),
                    include_patterns_raw: req.include_patterns_raw.as_deref(),
                    file_change_patterns_raw: req.file_change_patterns_raw.as_deref(),
                    backup_sources: req.backup_sources.as_deref(),
                    backup_sources_per_agent: req.backup_sources_per_agent.as_deref(),
                    exclude_patterns_per_agent: req.exclude_patterns_per_agent.as_deref(),
                    include_patterns_per_agent: req.include_patterns_per_agent.as_deref(),
                    file_change_patterns_per_agent: req
                        .file_change_patterns_per_agent
                        .as_deref(),
                    commands_per_agent: req.commands_per_agent.as_deref(),
                }
            }
        }
    )+};
}

schedule_text_fields_from!(CreateScheduleRequest, UpdateScheduleRequest);

impl ScheduleTextFields<'_> {
    /// Rejects the first string that exceeds its [`MaxLen`] cap.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::BadRequest`] naming the offending field.
    pub(super) fn validate(&self) -> Result<(), ApiError> {
        helpers::validate_opt_max_len(self.name, "name", MaxLen::Name)?;
        helpers::validate_max_len(self.cron_expression, "cron_expression", MaxLen::Name)?;
        helpers::validate_opt_max_len(
            self.exclude_patterns_raw,
            "exclude_patterns_raw",
            MaxLen::Text,
        )?;
        helpers::validate_opt_max_len(
            self.include_patterns_raw,
            "include_patterns_raw",
            MaxLen::Text,
        )?;
        helpers::validate_opt_max_len(
            self.file_change_patterns_raw,
            "file_change_patterns_raw",
            MaxLen::Text,
        )?;
        self.backup_sources.map_or(Ok(()), |paths| {
            helpers::validate_each_max_len(paths, "backup_sources", MaxLen::Path)
        })?;
        self.backup_sources_per_agent
            .unwrap_or_default()
            .iter()
            .enumerate()
            .try_for_each(|(i, entry)| {
                helpers::validate_each_max_len(
                    &entry.paths,
                    &format!("backup_sources_per_agent[{i}].paths"),
                    MaxLen::Path,
                )
            })?;
        validate_each_raw_text(
            self.exclude_patterns_per_agent
                .unwrap_or_default()
                .iter()
                .map(|entry| entry.raw_text.as_str()),
            "exclude_patterns_per_agent",
        )?;
        validate_each_raw_text(
            self.include_patterns_per_agent
                .unwrap_or_default()
                .iter()
                .map(|entry| entry.raw_text.as_str()),
            "include_patterns_per_agent",
        )?;
        validate_each_raw_text(
            self.file_change_patterns_per_agent
                .unwrap_or_default()
                .iter()
                .map(|entry| entry.raw_text.as_str()),
            "file_change_patterns_per_agent",
        )?;
        // Checked here rather than only as each override is stored, so an
        // over-long command cannot leave the schedule and the overrides before
        // it already written when the request is refused.
        self.commands_per_agent
            .unwrap_or_default()
            .iter()
            .enumerate()
            .try_for_each(|(i, entry)| {
                helpers::validate_each_command_max_len(
                    &entry.pre_backup_commands,
                    &format!("commands_per_agent[{i}].pre_backup_commands"),
                )?;
                helpers::validate_each_command_max_len(
                    &entry.post_backup_commands,
                    &format!("commands_per_agent[{i}].post_backup_commands"),
                )
            })
    }
}

/// Caps each per-agent override's `raw_text`, naming the entry by its index
/// in `field_name` (e.g. `exclude_patterns_per_agent[2].raw_text`).
fn validate_each_raw_text<'a>(
    raw_texts: impl Iterator<Item = &'a str>,
    field_name: &str,
) -> Result<(), ApiError> {
    raw_texts.enumerate().try_for_each(|(i, raw_text)| {
        helpers::validate_max_len(
            raw_text,
            &format!("{field_name}[{i}].raw_text"),
            MaxLen::Text,
        )
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::ScheduleTextFields;
    use crate::{
        api::{
            helpers::MaxLen,
            schedules::{CreateScheduleRequest, UpdateScheduleRequest},
        },
        error::ApiError,
    };

    fn create(extra: &Value) -> CreateScheduleRequest {
        let mut body = json!({
            "agent_ids": [1],
            "repo_id": 1,
            "cron_expression": "0 2 * * *",
        });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(body).unwrap()
    }

    fn update(extra: &Value) -> UpdateScheduleRequest {
        let mut body = json!({ "cron_expression": "0 2 * * *" });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(body).unwrap()
    }

    fn rejected_field(result: Result<(), ApiError>) -> String {
        match result {
            Err(ApiError::BadRequest(message)) => message,
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    fn over(max: MaxLen) -> String {
        "a".repeat(max.chars().saturating_add(1))
    }

    fn at(max: MaxLen) -> String {
        "a".repeat(max.chars())
    }

    #[test]
    fn strings_at_their_limits_are_accepted() {
        let extra = json!({
            "name": at(MaxLen::Name),
            "exclude_patterns_raw": at(MaxLen::Text),
            "include_patterns_raw": at(MaxLen::Text),
            "file_change_patterns_raw": at(MaxLen::Text),
            "backup_sources": [at(MaxLen::Path)],
            "backup_sources_per_agent": [{ "agent_id": 1, "paths": [at(MaxLen::Path)] }],
            "exclude_patterns_per_agent": [{ "agent_id": 1, "raw_text": at(MaxLen::Text) }],
            "include_patterns_per_agent": [{ "agent_id": 1, "raw_text": at(MaxLen::Text) }],
            "file_change_patterns_per_agent": [{ "agent_id": 1, "raw_text": at(MaxLen::Text) }],
            "commands_per_agent": [{
                "agent_id": 1,
                "pre_backup_commands": [{ "command": at(MaxLen::Text) }],
                "post_backup_commands": [{ "command": at(MaxLen::Text) }],
            }],
        });
        assert!(ScheduleTextFields::from(&create(&extra)).validate().is_ok());
        assert!(ScheduleTextFields::from(&update(&extra)).validate().is_ok());
    }

    #[test]
    fn each_over_limit_field_is_rejected_by_name() {
        let cases = [
            (json!({ "name": over(MaxLen::Name) }), "name "),
            (
                json!({ "cron_expression": over(MaxLen::Name) }),
                "cron_expression ",
            ),
            (
                json!({ "exclude_patterns_raw": over(MaxLen::Text) }),
                "exclude_patterns_raw ",
            ),
            (
                json!({ "include_patterns_raw": over(MaxLen::Text) }),
                "include_patterns_raw ",
            ),
            (
                json!({ "file_change_patterns_raw": over(MaxLen::Text) }),
                "file_change_patterns_raw ",
            ),
            (
                json!({ "backup_sources": ["/etc", over(MaxLen::Path)] }),
                "backup_sources[1] ",
            ),
            (
                json!({ "backup_sources_per_agent": [{ "agent_id": 1, "paths": [over(MaxLen::Path)] }] }),
                "backup_sources_per_agent[0].paths[0] ",
            ),
            (
                json!({ "exclude_patterns_per_agent": [{ "agent_id": 1, "raw_text": over(MaxLen::Text) }] }),
                "exclude_patterns_per_agent[0].raw_text ",
            ),
            (
                json!({ "include_patterns_per_agent": [{ "agent_id": 1, "raw_text": over(MaxLen::Text) }] }),
                "include_patterns_per_agent[0].raw_text ",
            ),
            (
                json!({ "file_change_patterns_per_agent": [{ "agent_id": 1, "raw_text": over(MaxLen::Text) }] }),
                "file_change_patterns_per_agent[0].raw_text ",
            ),
            (
                json!({ "backup_sources_per_agent": [
                    { "agent_id": 1, "paths": ["/etc"] },
                    { "agent_id": 2, "paths": ["/srv", over(MaxLen::Path)] },
                ] }),
                "backup_sources_per_agent[1].paths[1] ",
            ),
            (
                json!({ "exclude_patterns_per_agent": [
                    { "agent_id": 1, "raw_text": "*.tmp" },
                    { "agent_id": 2, "raw_text": over(MaxLen::Text) },
                ] }),
                "exclude_patterns_per_agent[1].raw_text ",
            ),
            (
                json!({ "commands_per_agent": [{
                    "agent_id": 1,
                    "pre_backup_commands": [
                        { "command": "true" },
                        { "command": over(MaxLen::Text) },
                    ],
                    "post_backup_commands": [],
                }] }),
                "commands_per_agent[0].pre_backup_commands[1] ",
            ),
            (
                json!({ "commands_per_agent": [
                    { "agent_id": 1, "pre_backup_commands": [], "post_backup_commands": [] },
                    {
                        "agent_id": 2,
                        "pre_backup_commands": [],
                        "post_backup_commands": [{ "command": over(MaxLen::Text) }],
                    },
                ] }),
                "commands_per_agent[1].post_backup_commands[0] ",
            ),
        ];
        for (extra, field) in cases {
            for message in [
                rejected_field(ScheduleTextFields::from(&create(&extra)).validate()),
                rejected_field(ScheduleTextFields::from(&update(&extra)).validate()),
            ] {
                assert!(message.starts_with(field), "{field}: {message}");
            }
        }
    }
}
