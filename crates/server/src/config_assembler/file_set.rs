// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The file set a backup schedule reads on one host.
//!
//! A host defines a file set under Sources > Files: paths, excludes, include
//! patterns, file change patterns and the commands that run around the
//! backup. A schedule takes that set as it is, defines its own set of the
//! same fields that replaces the host's as a whole, or reads no files at all
//! (see [`ScheduleFilesMode`]). Nothing from the two is ever combined.

use shared::{
    hooks::HookCommand,
    types::{FileChangePattern, ScheduleFilesMode},
};
use sqlx::PgPool;

use super::{parse_raw_file_change_patterns, parse_raw_pattern_lines};
use crate::{db, error::ApiError};

/// What one backup reads as files and the commands that run around it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct FileSet {
    pub paths: Vec<String>,
    pub excludes: Vec<String>,
    pub includes: Vec<String>,
    pub file_change_patterns: Vec<FileChangePattern>,
    pub pre_backup_commands: Vec<HookCommand>,
    pub post_backup_commands: Vec<HookCommand>,
}

/// A schedule's own values for one host: the host's per-host override where
/// the schedule has one, else the schedule-wide value. Only paths can be
/// split per host in the UI; the other per-host rows are kept from schedules
/// whose hosts had different sets before files modes existed.
#[derive(Debug, Default)]
pub(super) struct OwnOverrides {
    pub paths: Vec<String>,
    pub excludes_raw: Option<String>,
    pub includes_raw: Option<String>,
    pub file_change_patterns_raw: Option<String>,
    pub commands: Option<db::PerAgentCommands>,
}

impl FileSet {
    /// The host's own file set, as its Sources > Files defines it.
    pub(super) fn of_host(agent: &db::AgentRow) -> Self {
        Self {
            paths: agent.default_backup_paths.clone(),
            excludes: agent.default_exclude_patterns.clone(),
            includes: agent.default_include_patterns.clone(),
            file_change_patterns: parse_raw_file_change_patterns(
                &agent.default_file_change_patterns_raw,
            ),
            pre_backup_commands: agent.default_pre_backup_commands.0.clone(),
            post_backup_commands: agent.default_post_backup_commands.0.clone(),
        }
    }

    /// The schedule's own file set on one host.
    pub(super) fn of_schedule(schedule: &db::ScheduleRow, overrides: OwnOverrides) -> Self {
        let (pre_backup_commands, post_backup_commands) = overrides.commands.map_or_else(
            || {
                (
                    schedule.pre_backup_commands.0.clone(),
                    schedule.post_backup_commands.0.clone(),
                )
            },
            |c| (c.pre_backup_commands, c.post_backup_commands),
        );
        Self {
            paths: overrides.paths,
            excludes: parse_raw_pattern_lines(
                overrides
                    .excludes_raw
                    .as_deref()
                    .unwrap_or(&schedule.exclude_patterns_raw),
            ),
            includes: parse_raw_pattern_lines(
                overrides
                    .includes_raw
                    .as_deref()
                    .unwrap_or(&schedule.include_patterns_raw),
            ),
            file_change_patterns: parse_raw_file_change_patterns(
                overrides
                    .file_change_patterns_raw
                    .as_deref()
                    .unwrap_or(&schedule.file_change_patterns_raw),
            ),
            pre_backup_commands,
            post_backup_commands,
        }
    }

    /// The same set with its pre- and post-backup commands left out.
    pub(super) fn without_commands(self) -> Self {
        Self {
            pre_backup_commands: Vec::new(),
            post_backup_commands: Vec::new(),
            ..self
        }
    }

    /// The excludes borg runs with: the server-wide list unless the schedule
    /// ignores it, then the set's own, each pattern once.
    pub(super) fn exclude_patterns(
        &self,
        global_excludes: &[String],
        ignore_global: bool,
    ) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        global_excludes
            .iter()
            .filter(|_| !ignore_global)
            .chain(&self.excludes)
            .filter(|pattern| seen.insert(pattern.as_str()))
            .cloned()
            .collect()
    }
}

/// The file set `schedule` reads on `agent`.
pub(super) async fn resolve(
    pool: &PgPool,
    schedule: &db::ScheduleRow,
    agent: &db::AgentRow,
) -> Result<FileSet, ApiError> {
    match schedule.files_mode {
        ScheduleFilesMode::Host if schedule.run_host_commands => Ok(FileSet::of_host(agent)),
        ScheduleFilesMode::Host => Ok(FileSet::of_host(agent).without_commands()),
        ScheduleFilesMode::Own => Ok(FileSet::of_schedule(
            schedule,
            own_overrides(pool, schedule.id, agent.id).await?,
        )),
        ScheduleFilesMode::Off => Ok(FileSet::default()),
    }
}

async fn own_overrides(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
) -> Result<OwnOverrides, ApiError> {
    let per_agent_paths =
        db::list_backup_sources_for_schedule_agent(pool, schedule_id, agent_id).await?;
    let paths = if per_agent_paths.is_empty() {
        db::list_backup_sources_for_schedule(pool, schedule_id).await?
    } else {
        per_agent_paths
    };
    Ok(OwnOverrides {
        paths,
        excludes_raw: db::get_per_agent_excludes_raw(pool, schedule_id, agent_id).await?,
        includes_raw: db::get_per_agent_includes_raw(pool, schedule_id, agent_id).await?,
        file_change_patterns_raw: db::get_per_agent_file_change_patterns_raw(
            pool,
            schedule_id,
            agent_id,
        )
        .await?,
        commands: db::get_per_agent_commands(pool, schedule_id, agent_id).await?,
    })
}

#[cfg(test)]
mod tests;
