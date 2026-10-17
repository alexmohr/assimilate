// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Turns what the server knows into metric families.
//!
//! Everything here is a pure function of a [`Snapshot`], so what each metric
//! reports - and, as importantly, when it reports nothing - is tested without a
//! database or a connected agent.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use shared::types::{BackupStatus, ReportStatus, ScheduleType};

use super::exposition::{Family, Sample};
use crate::{
    db::metrics::{AgentMetricsRow, RepositoryMetricsRow, ScheduleTargetMetricsRow},
    repo_op_tracker::RepoOpTotals,
};

/// Label naming the agent's host: its hostname, plus its domain when one is
/// set, so two hosts sharing a hostname stay two series.
pub const LABEL_HOST: &str = "host";
/// Label naming a schedule by its display name.
pub const LABEL_SCHEDULE: &str = "schedule";
/// Label carrying a schedule's ID: schedule names need not be unique.
pub const LABEL_SCHEDULE_ID: &str = "schedule_id";
/// Label naming a repository.
pub const LABEL_REPOSITORY: &str = "repository";
/// Label naming the host a repository lives on.
pub const LABEL_REPOSITORY_HOST: &str = "repository_host";

/// The finished-run outcomes `assimilate_backup_last_run_status` has a series
/// for, so a target always reports all three and only the current one is `1`.
const OUTCOMES: [BackupStatus; 3] = [
    BackupStatus::Success,
    BackupStatus::Warning,
    BackupStatus::Failed,
];

/// Everything one scrape is rendered from.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// When the snapshot was taken.
    pub now: DateTime<Utc>,
    /// The timezone schedules' cron expressions are evaluated in.
    pub timezone: chrono_tz::Tz,
    /// Every schedule target.
    pub targets: Vec<ScheduleTargetMetricsRow>,
    /// Every repository.
    pub repositories: Vec<RepositoryMetricsRow>,
    /// Every agent.
    pub agents: Vec<AgentMetricsRow>,
    /// IDs of the agents connected right now.
    pub connected_agents: HashSet<i64>,
    /// When the scheduler last finished a tick, if it has since start-up.
    pub scheduler_last_tick: Option<DateTime<Utc>>,
    /// Running and queued repository operations.
    pub jobs: RepoOpTotals,
    /// This server's version.
    pub server_version: String,
}

fn host_label(hostname: &str, domain: Option<&str>) -> String {
    domain.filter(|domain| !domain.is_empty()).map_or_else(
        || hostname.to_owned(),
        |domain| format!("{hostname}.{domain}"),
    )
}

fn target_labels(sample: Sample, target: &ScheduleTargetMetricsRow) -> Sample {
    sample
        .label(LABEL_SCHEDULE_ID, target.schedule_id.to_string())
        .label(LABEL_SCHEDULE, target.schedule_name.as_str())
        .label(
            LABEL_HOST,
            host_label(&target.hostname, target.domain.as_deref()),
        )
        .label(LABEL_REPOSITORY, target.repo_name.as_str())
        .label(LABEL_REPOSITORY_HOST, target.repo_host.as_str())
}

fn repository_labels(sample: Sample, repo: &RepositoryMetricsRow) -> Sample {
    sample
        .label(LABEL_REPOSITORY, repo.repo_name.as_str())
        .label(LABEL_REPOSITORY_HOST, repo.repo_host.as_str())
}

fn agent_labels(sample: Sample, agent: &AgentMetricsRow) -> Sample {
    sample.label(
        LABEL_HOST,
        host_label(&agent.hostname, agent.domain.as_deref()),
    )
}

/// The deadline after which a backup target counts as overdue - the next
/// scheduled run after its last success, plus the dashboard's grace period.
/// Unknown for a disabled schedule, a target that never succeeded, and a cron
/// expression that does not parse.
fn backup_deadline(
    target: &ScheduleTargetMetricsRow,
    timezone: chrono_tz::Tz,
) -> Option<DateTime<Utc>> {
    if !target.schedule_enabled {
        return None;
    }
    target.last_success_at.and_then(|last_success| {
        crate::api::stats::run_deadline_after(&target.cron_expression, last_success, timezone)
    })
}

fn schedule_target_families(snapshot: &Snapshot) -> Vec<Family> {
    let mut enabled = Family::gauge(
        "assimilate_schedule_enabled",
        "Whether the schedule is enabled (1) or disabled (0), per target.",
    );
    let mut last_success = Family::gauge(
        "assimilate_backup_last_success_timestamp_seconds",
        "Unix time the target's latest successful backup finished. Absent until one has.",
    );
    let mut last_status = Family::gauge(
        "assimilate_backup_last_run_status",
        "Outcome of the target's latest finished backup: 1 for the status it ended with, 0 for \
         the others. Absent until a backup has finished.",
    );
    let mut deadline = Family::gauge(
        "assimilate_backup_deadline_timestamp_seconds",
        "Unix time after which the target is overdue: the next scheduled run after its latest \
         success, plus a 30 minute grace period.",
    );
    let mut overdue = Family::gauge(
        "assimilate_backup_overdue_seconds",
        "Seconds the target is past its deadline, 0 while it is not overdue.",
    );

    snapshot.targets.iter().for_each(|target| {
        enabled.push(
            target_labels(Sample::new(i64::from(target.schedule_enabled)), target)
                .label("type", target.schedule_type.to_string()),
        );

        if !matches!(target.schedule_type, ScheduleType::Backup) {
            return;
        }
        last_success.push_opt(target.last_success_at.map(|at| at.timestamp()), |s| {
            target_labels(s, target)
        });
        if let Some(outcome) = target.latest_status.and_then(ReportStatus::outcome) {
            for candidate in OUTCOMES {
                last_status.push(
                    target_labels(Sample::new(i64::from(candidate == outcome)), target)
                        .label("status", candidate.to_string()),
                );
            }
        }
        if let Some(due) = backup_deadline(target, snapshot.timezone) {
            deadline.push(target_labels(Sample::new(due.timestamp()), target));
            overdue.push(target_labels(
                Sample::new(snapshot.now.signed_duration_since(due).num_seconds().max(0)),
                target,
            ));
        }
    });

    vec![enabled, last_success, last_status, deadline, overdue]
}

fn repository_families(snapshot: &Snapshot) -> Vec<Family> {
    let mut size = Family::gauge(
        "assimilate_repository_size_bytes",
        "Total size of every archive in the repository before compression and deduplication.",
    );
    let mut deduplicated = Family::gauge(
        "assimilate_repository_deduplicated_size_bytes",
        "Size of the repository after compression and deduplication; quotas are measured against \
         this.",
    );
    let mut archives = Family::gauge(
        "assimilate_repository_archives",
        "Number of archives in the repository.",
    );
    let mut updated = Family::gauge(
        "assimilate_repository_stats_updated_timestamp_seconds",
        "Unix time the repository statistics were last read from borg.",
    );
    let mut warn = Family::gauge(
        "assimilate_repository_quota_warn_bytes",
        "The repository quota's warn threshold. Absent without an enabled quota that sets one.",
    );
    let mut critical = Family::gauge(
        "assimilate_repository_quota_critical_bytes",
        "The repository quota's critical threshold. Absent without an enabled quota that sets one.",
    );

    snapshot.repositories.iter().for_each(|repo| {
        size.push_opt(repo.original_size, |s| repository_labels(s, repo));
        deduplicated.push_opt(repo.deduplicated_size, |s| repository_labels(s, repo));
        archives.push_opt(repo.archive_count.map(i64::from), |s| {
            repository_labels(s, repo)
        });
        updated.push_opt(repo.stats_updated_at.map(|at| at.timestamp()), |s| {
            repository_labels(s, repo)
        });
        warn.push_opt(repo.quota_warn_bytes, |s| repository_labels(s, repo));
        critical.push_opt(repo.quota_critical_bytes, |s| repository_labels(s, repo));
    });

    vec![size, deduplicated, archives, updated, warn, critical]
}

fn agent_families(snapshot: &Snapshot) -> Vec<Family> {
    let mut connected = Family::gauge(
        "assimilate_agent_connected",
        "Whether the host's agent is connected to the server (1) or not (0).",
    );
    let mut info = Family::gauge(
        "assimilate_agent_info",
        "Always 1; the labels carry the agent version and the borg version from its latest backup \
         report. A label is left out while its value is unknown.",
    );
    let mut last_seen = Family::gauge(
        "assimilate_agent_last_seen_timestamp_seconds",
        "Unix time the host's agent was last seen by the server.",
    );

    snapshot.agents.iter().for_each(|agent| {
        connected.push(agent_labels(
            Sample::new(i64::from(
                snapshot.connected_agents.contains(&agent.agent_id),
            )),
            agent,
        ));
        info.push(
            agent_labels(Sample::new(1), agent)
                .label_opt("version", agent.agent_version.as_deref())
                .label_opt("borg_version", agent.borg_version.as_deref()),
        );
        last_seen.push_opt(agent.last_seen_at.map(|at| at.timestamp()), |s| {
            agent_labels(s, agent)
        });
    });

    vec![connected, info, last_seen]
}

fn server_families(snapshot: &Snapshot) -> Vec<Family> {
    let mut build = Family::gauge(
        "assimilate_build_info",
        "Always 1; the version label carries the server version.",
    );
    build.push(Sample::new(1).label("version", snapshot.server_version.as_str()));

    let mut last_tick = Family::gauge(
        "assimilate_scheduler_last_tick_timestamp_seconds",
        "Unix time the scheduler last finished a pass over the schedules. Absent until its first \
         pass after the server started.",
    );
    last_tick.push_opt(snapshot.scheduler_last_tick.map(|at| at.timestamp()), |s| s);

    let mut running = Family::gauge(
        "assimilate_jobs_running",
        "Repositories with an operation (backup, check, sync, ...) running right now.",
    );
    running.push(Sample::new(i64::from(snapshot.jobs.running)));

    let mut queued = Family::gauge(
        "assimilate_jobs_queued",
        "Repository operations waiting for the one running on their repository to finish.",
    );
    queued.push(Sample::new(i64::from(snapshot.jobs.queued)));

    vec![build, last_tick, running, queued]
}

/// Every metric family the snapshot supports, in a stable order.
#[must_use]
pub fn families(snapshot: &Snapshot) -> Vec<Family> {
    [
        server_families(snapshot),
        agent_families(snapshot),
        schedule_target_families(snapshot),
        repository_families(snapshot),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;
    use crate::metrics::exposition::render;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 17, hour, 0, 0).unwrap()
    }

    fn target() -> ScheduleTargetMetricsRow {
        ScheduleTargetMetricsRow {
            schedule_id: 7,
            schedule_name: "Nightly".to_owned(),
            schedule_type: ScheduleType::Backup,
            schedule_enabled: true,
            cron_expression: "0 2 * * *".to_owned(),
            hostname: "web-01".to_owned(),
            domain: None,
            repo_name: "offsite".to_owned(),
            repo_host: "backup.example.com".to_owned(),
            latest_status: None,
            last_success_at: None,
        }
    }

    fn repository() -> RepositoryMetricsRow {
        RepositoryMetricsRow {
            repo_name: "offsite".to_owned(),
            repo_host: "backup.example.com".to_owned(),
            original_size: None,
            deduplicated_size: None,
            archive_count: None,
            stats_updated_at: None,
            quota_warn_bytes: None,
            quota_critical_bytes: None,
        }
    }

    fn agent(agent_id: i64) -> AgentMetricsRow {
        AgentMetricsRow {
            agent_id,
            hostname: "web-01".to_owned(),
            domain: None,
            agent_version: None,
            borg_version: None,
            last_seen_at: None,
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            now: at(12),
            timezone: chrono_tz::UTC,
            targets: Vec::new(),
            repositories: Vec::new(),
            agents: Vec::new(),
            connected_agents: HashSet::new(),
            scheduler_last_tick: None,
            jobs: RepoOpTotals::default(),
            server_version: "1.2.3".to_owned(),
        }
    }

    fn text(snapshot: &Snapshot) -> String {
        render(&families(snapshot))
    }

    const TARGET_LABELS: &str = "schedule_id=\"7\",schedule=\"Nightly\",host=\"web-01\",\
                                 repository=\"offsite\",repository_host=\"backup.example.com\"";

    #[test]
    fn a_target_that_never_ran_reports_only_that_its_schedule_is_enabled() {
        let snapshot = Snapshot {
            targets: vec![target()],
            ..snapshot()
        };

        let text = text(&snapshot);

        assert!(text.contains(&format!(
            "assimilate_schedule_enabled{{{TARGET_LABELS},type=\"backup\"}} 1\n"
        )));
        assert!(!text.contains("assimilate_backup_last_success_timestamp_seconds"));
        assert!(!text.contains("assimilate_backup_last_run_status"));
        assert!(!text.contains("assimilate_backup_overdue_seconds"));
        assert!(!text.contains("assimilate_backup_deadline_timestamp_seconds"));
    }

    #[test]
    fn a_successful_target_reports_its_success_status_and_deadline() {
        let snapshot = Snapshot {
            targets: vec![ScheduleTargetMetricsRow {
                latest_status: Some(ReportStatus::Success),
                last_success_at: Some(at(2)),
                ..target()
            }],
            ..snapshot()
        };

        let text = text(&snapshot);

        assert!(text.contains(&format!(
            "assimilate_backup_last_success_timestamp_seconds{{{TARGET_LABELS}}} {}\n",
            at(2).timestamp()
        )));
        assert!(text.contains(&format!(
            "assimilate_backup_last_run_status{{{TARGET_LABELS},status=\"success\"}} 1\n"
        )));
        assert!(text.contains(&format!(
            "assimilate_backup_last_run_status{{{TARGET_LABELS},status=\"failed\"}} 0\n"
        )));
        let deadline = Utc.with_ymd_and_hms(2026, 10, 18, 2, 30, 0).unwrap();
        assert!(text.contains(&format!(
            "assimilate_backup_deadline_timestamp_seconds{{{TARGET_LABELS}}} {}\n",
            deadline.timestamp()
        )));
        assert!(text.contains(&format!(
            "assimilate_backup_overdue_seconds{{{TARGET_LABELS}}} 0\n"
        )));
    }

    #[test]
    fn a_target_past_its_deadline_reports_how_far_past_it_is() {
        let snapshot = Snapshot {
            now: Utc.with_ymd_and_hms(2026, 10, 18, 3, 30, 0).unwrap(),
            targets: vec![ScheduleTargetMetricsRow {
                latest_status: Some(ReportStatus::Failed),
                last_success_at: Some(at(2)),
                ..target()
            }],
            ..snapshot()
        };

        let text = text(&snapshot);

        assert!(text.contains(&format!(
            "assimilate_backup_overdue_seconds{{{TARGET_LABELS}}} 3600\n"
        )));
        assert!(text.contains(&format!(
            "assimilate_backup_last_run_status{{{TARGET_LABELS},status=\"failed\"}} 1\n"
        )));
        assert!(text.contains(&format!(
            "assimilate_backup_last_run_status{{{TARGET_LABELS},status=\"success\"}} 0\n"
        )));
    }

    #[test]
    fn a_disabled_schedule_has_no_deadline() {
        let snapshot = Snapshot {
            targets: vec![ScheduleTargetMetricsRow {
                schedule_enabled: false,
                last_success_at: Some(at(2)),
                ..target()
            }],
            ..snapshot()
        };

        let text = text(&snapshot);

        assert!(text.contains(&format!(
            "assimilate_schedule_enabled{{{TARGET_LABELS},type=\"backup\"}} 0\n"
        )));
        assert!(text.contains("assimilate_backup_last_success_timestamp_seconds"));
        assert!(!text.contains("assimilate_backup_overdue_seconds"));
    }

    #[test]
    fn an_unparseable_cron_expression_has_no_deadline() {
        let snapshot = Snapshot {
            targets: vec![ScheduleTargetMetricsRow {
                cron_expression: "not cron".to_owned(),
                last_success_at: Some(at(2)),
                ..target()
            }],
            ..snapshot()
        };

        assert!(!text(&snapshot).contains("assimilate_backup_overdue_seconds"));
    }

    #[test]
    fn a_running_or_cancelled_latest_run_is_not_a_last_run_status() {
        let snapshot = Snapshot {
            targets: vec![ScheduleTargetMetricsRow {
                latest_status: Some(ReportStatus::Started),
                ..target()
            }],
            ..snapshot()
        };

        assert!(!text(&snapshot).contains("assimilate_backup_last_run_status"));
    }

    #[test]
    fn a_check_schedule_reports_only_whether_it_is_enabled() {
        let snapshot = Snapshot {
            targets: vec![ScheduleTargetMetricsRow {
                schedule_type: ScheduleType::Check,
                last_success_at: Some(at(2)),
                latest_status: Some(ReportStatus::Success),
                ..target()
            }],
            ..snapshot()
        };

        let text = text(&snapshot);

        assert!(text.contains("type=\"check\"} 1\n"));
        assert!(!text.contains("assimilate_backup_"));
    }

    #[test]
    fn a_host_with_a_domain_is_labelled_by_its_full_name() {
        let snapshot = Snapshot {
            agents: vec![AgentMetricsRow {
                domain: Some("dc1.example.com".to_owned()),
                ..agent(1)
            }],
            ..snapshot()
        };

        assert!(
            text(&snapshot)
                .contains("assimilate_agent_connected{host=\"web-01.dc1.example.com\"} 0\n")
        );
    }

    #[test]
    fn agents_report_connection_and_only_the_versions_they_are_known_to_run() {
        let snapshot = Snapshot {
            agents: vec![
                AgentMetricsRow {
                    agent_version: Some("0.9.1".to_owned()),
                    borg_version: Some("1.4.0".to_owned()),
                    last_seen_at: Some(at(11)),
                    ..agent(1)
                },
                AgentMetricsRow {
                    hostname: "db-01".to_owned(),
                    ..agent(2)
                },
            ],
            connected_agents: HashSet::from([1]),
            ..snapshot()
        };

        let text = text(&snapshot);

        assert!(text.contains("assimilate_agent_connected{host=\"web-01\"} 1\n"));
        assert!(text.contains("assimilate_agent_connected{host=\"db-01\"} 0\n"));
        assert!(text.contains(
            "assimilate_agent_info{host=\"web-01\",version=\"0.9.1\",borg_version=\"1.4.0\"} 1\n"
        ));
        assert!(text.contains("assimilate_agent_info{host=\"db-01\"} 1\n"));
        assert!(text.contains(&format!(
            "assimilate_agent_last_seen_timestamp_seconds{{host=\"web-01\"}} {}\n",
            at(11).timestamp()
        )));
        assert!(!text.contains("assimilate_agent_last_seen_timestamp_seconds{host=\"db-01\"}"));
    }

    #[test]
    fn a_repository_never_read_from_borg_reports_no_size() {
        let snapshot = Snapshot {
            repositories: vec![repository()],
            ..snapshot()
        };

        let text = text(&snapshot);

        assert!(!text.contains("assimilate_repository_"));
    }

    #[test]
    fn a_repository_reports_its_statistics_and_quota() {
        let snapshot = Snapshot {
            repositories: vec![RepositoryMetricsRow {
                original_size: Some(1000),
                deduplicated_size: Some(0),
                archive_count: Some(12),
                stats_updated_at: Some(at(1)),
                quota_warn_bytes: None,
                quota_critical_bytes: Some(5000),
                ..repository()
            }],
            ..snapshot()
        };

        let text = text(&snapshot);
        let labels = "repository=\"offsite\",repository_host=\"backup.example.com\"";

        assert!(text.contains(&format!(
            "assimilate_repository_size_bytes{{{labels}}} 1000\n"
        )));
        assert!(text.contains(&format!(
            "assimilate_repository_deduplicated_size_bytes{{{labels}}} 0\n"
        )));
        assert!(text.contains(&format!("assimilate_repository_archives{{{labels}}} 12\n")));
        assert!(text.contains(&format!(
            "assimilate_repository_quota_critical_bytes{{{labels}}} 5000\n"
        )));
        assert!(!text.contains("assimilate_repository_quota_warn_bytes"));
    }

    #[test]
    fn the_server_reports_jobs_and_omits_a_scheduler_that_has_not_ticked() {
        let snapshot = Snapshot {
            jobs: RepoOpTotals {
                running: 2,
                queued: 5,
            },
            ..snapshot()
        };

        let text = text(&snapshot);

        assert!(text.contains("assimilate_build_info{version=\"1.2.3\"} 1\n"));
        assert!(text.contains("assimilate_jobs_running 2\n"));
        assert!(text.contains("assimilate_jobs_queued 5\n"));
        assert!(!text.contains("assimilate_scheduler_last_tick_timestamp_seconds"));
    }

    #[test]
    fn the_scheduler_last_tick_is_reported_once_known() {
        let snapshot = Snapshot {
            scheduler_last_tick: Some(at(12)),
            ..snapshot()
        };

        assert!(text(&snapshot).contains(&format!(
            "assimilate_scheduler_last_tick_timestamp_seconds {}\n",
            at(12).timestamp()
        )));
    }

    #[test]
    fn label_values_from_names_are_escaped() {
        let snapshot = Snapshot {
            targets: vec![ScheduleTargetMetricsRow {
                schedule_name: "say \"nightly\"\\now".to_owned(),
                ..target()
            }],
            ..snapshot()
        };

        assert!(text(&snapshot).contains("schedule=\"say \\\"nightly\\\"\\\\now\""));
    }
}
