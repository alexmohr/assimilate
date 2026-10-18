// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use shared::{responses::HealthSummaryResponse, types::BackupStatus};

use crate::provision::{AdminSession, LocalServer, ProvisionError};

/// How this computer's backups are doing, as the tray icon shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupHealth {
    /// No enabled schedule backs anything up.
    NothingScheduled,
    /// Schedules exist, but none has finished a backup yet.
    Waiting,
    /// Every scheduled backup's last run succeeded and none is overdue.
    UpToDate,
    /// At least one scheduled backup needs looking at.
    NeedsAttention {
        /// Backups whose last finished run failed.
        failed: usize,
        /// Backups that are overdue, among those whose last run didn't fail.
        overdue: usize,
    },
}

impl BackupHealth {
    /// Sums up `/api/stats/health`: one entry per schedule and repository.
    ///
    /// A backup counts as failed by its last *finished* run, so a retry
    /// that is still running doesn't hide the failure. Disabled schedules
    /// are left out: nothing will run them, so an old failure there isn't
    /// news.
    #[must_use]
    pub fn from_entries(entries: &[HealthSummaryResponse]) -> Self {
        let scheduled: Vec<&HealthSummaryResponse> = entries
            .iter()
            .filter(|entry| entry.schedule_enabled != Some(false))
            .collect();
        let failed = scheduled
            .iter()
            .filter(|entry| entry.last_backup_status == Some(BackupStatus::Failed))
            .count();
        let overdue = scheduled
            .iter()
            .filter(|entry| entry.last_backup_status != Some(BackupStatus::Failed))
            .filter(|entry| entry.is_overdue)
            .count();
        if failed > 0 || overdue > 0 {
            Self::NeedsAttention { failed, overdue }
        } else if scheduled.is_empty() {
            Self::NothingScheduled
        } else if scheduled.iter().all(|entry| entry.last_backup_at.is_none()) {
            Self::Waiting
        } else {
            Self::UpToDate
        }
    }

    /// Whether the tray icon should draw attention to itself.
    #[must_use]
    pub fn needs_attention(self) -> bool {
        matches!(self, Self::NeedsAttention { .. })
    }

    /// The tray icon's tooltip.
    #[must_use]
    pub fn tooltip(self) -> String {
        match self {
            Self::NothingScheduled => "Assimilate: no backups scheduled".to_owned(),
            Self::Waiting => "Assimilate: waiting for the first backup".to_owned(),
            Self::UpToDate => "Assimilate: backups are up to date".to_owned(),
            Self::NeedsAttention { failed, overdue } => {
                let parts: Vec<String> = [(failed, "failed"), (overdue, "overdue")]
                    .into_iter()
                    .filter(|&(count, _)| count > 0)
                    .map(|(count, what)| format!("{} {what}", backups(count)))
                    .collect();
                format!("Assimilate: {}", parts.join(", "))
            }
        }
    }
}

/// "1 backup", "2 backups".
fn backups(count: usize) -> String {
    if count == 1 {
        "1 backup".to_owned()
    } else {
        format!("{count} backups")
    }
}

/// Asks the local server how its backups are doing.
#[derive(Debug, Clone)]
pub struct BackupStatusSource {
    server: LocalServer,
    session: AdminSession,
}

impl BackupStatusSource {
    /// A source that asks `server` as the signed-in admin.
    #[must_use]
    pub fn new(server: LocalServer, session: AdminSession) -> Self {
        Self { server, session }
    }

    /// The current [`BackupHealth`].
    ///
    /// # Errors
    ///
    /// Fails if the server can't be reached or refuses the request.
    pub async fn fetch(&self) -> Result<BackupHealth, ProvisionError> {
        let entries = self.server.health_summary(&self.session).await?;
        Ok(BackupHealth::from_entries(&entries))
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn entry(
        status: Option<BackupStatus>,
        overdue: bool,
        enabled: Option<bool>,
    ) -> HealthSummaryResponse {
        HealthSummaryResponse {
            repo_id: 1,
            schedule_id: 1,
            hostname: "laptop".to_owned(),
            target_name: "home".to_owned(),
            last_status: status,
            last_backup_at: status.map(|_| Utc::now()),
            last_backup_status: status,
            last_success_at: None,
            is_overdue: overdue,
            last_error_message: None,
            cron_expression: Some("0 2 * * *".to_owned()),
            schedule_enabled: enabled,
            consecutive_missed_backups: 0,
            missed_backup_threshold: 3,
        }
    }

    const OK: Option<BackupStatus> = Some(BackupStatus::Success);
    const FAILED: Option<BackupStatus> = Some(BackupStatus::Failed);
    const ENABLED: Option<bool> = Some(true);

    #[test]
    fn nothing_scheduled_without_enabled_schedules() {
        assert_eq!(
            BackupHealth::from_entries(&[]),
            BackupHealth::NothingScheduled
        );
        assert_eq!(
            BackupHealth::from_entries(&[entry(OK, false, Some(false))]),
            BackupHealth::NothingScheduled
        );
    }

    #[test]
    fn waiting_until_a_backup_has_finished() {
        assert_eq!(
            BackupHealth::from_entries(&[entry(None, false, ENABLED)]),
            BackupHealth::Waiting
        );
    }

    #[test]
    fn up_to_date_when_every_backup_succeeded() {
        let entries = [
            entry(OK, false, ENABLED),
            entry(Some(BackupStatus::Warning), false, None),
        ];

        assert_eq!(BackupHealth::from_entries(&entries), BackupHealth::UpToDate);
    }

    #[test]
    fn counts_failed_and_overdue_backups() {
        let entries = [
            entry(FAILED, false, ENABLED),
            // Failed and overdue counts once, as failed.
            entry(FAILED, true, ENABLED),
            entry(OK, true, ENABLED),
            entry(OK, false, ENABLED),
        ];

        assert_eq!(
            BackupHealth::from_entries(&entries),
            BackupHealth::NeedsAttention {
                failed: 2,
                overdue: 1
            }
        );
    }

    #[test]
    fn a_failure_counts_by_the_last_finished_run() {
        // A retry is running (no outcome yet) after the last run failed.
        let mut retrying = entry(FAILED, false, ENABLED);
        retrying.last_status = None;

        assert!(BackupHealth::from_entries(&[retrying]).needs_attention());
    }

    #[test]
    fn a_disabled_schedule_never_needs_attention() {
        let entries = [entry(FAILED, true, Some(false)), entry(OK, false, ENABLED)];

        assert_eq!(BackupHealth::from_entries(&entries), BackupHealth::UpToDate);
    }

    #[test]
    fn the_tooltip_names_what_needs_attention() {
        let tooltip = |failed, overdue| BackupHealth::NeedsAttention { failed, overdue }.tooltip();

        assert_eq!(tooltip(1, 0), "Assimilate: 1 backup failed");
        assert_eq!(tooltip(0, 2), "Assimilate: 2 backups overdue");
        assert_eq!(
            tooltip(2, 1),
            "Assimilate: 2 backups failed, 1 backup overdue"
        );
        assert_eq!(
            BackupHealth::UpToDate.tooltip(),
            "Assimilate: backups are up to date"
        );
    }
}
