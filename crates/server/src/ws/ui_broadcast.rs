// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use shared::protocol::ServerToUi;
use tokio::sync::broadcast;

const CHANNEL_CAPACITY: usize = 128;

#[derive(Debug, Clone)]
pub struct ImportProgressSnapshot {
    pub progress: i32,
    pub total: i32,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ActiveBackupSnapshot {
    pub hostname: String,
    pub target_name: String,
    pub archive_name: Option<String>,
    pub schedule_id: Option<i64>,
    pub repo_id: i64,
    pub progress_line: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UiBroadcast {
    sender: Arc<broadcast::Sender<ServerToUi>>,
    import_progress: Arc<RwLock<HashMap<i64, ImportProgressSnapshot>>>,
    active_backups: Arc<RwLock<HashMap<i64, ActiveBackupSnapshot>>>,
}

impl Default for UiBroadcast {
    fn default() -> Self {
        Self::new()
    }
}

impl UiBroadcast {
    #[must_use]
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            sender: Arc::new(sender),
            import_progress: Arc::new(RwLock::new(HashMap::new())),
            active_backups: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn send(&self, event: ServerToUi) {
        match &event {
            ServerToUi::ImportProgress {
                repo_id,
                progress,
                total,
                message,
            } => {
                self.set_import_progress(
                    *repo_id,
                    ImportProgressSnapshot {
                        progress: *progress,
                        total: *total,
                        message: message.clone(),
                    },
                );
            }
            ServerToUi::BackupCompleted { report, .. } => {
                self.clear_active_backup(report.repo_id.0);
            }
            ServerToUi::BackupLog {
                hostname,
                schedule_id,
                repo_id,
                line,
            } => {
                self.update_active_backup_progress(*repo_id, hostname, *schedule_id, line);
            }
            _ => {}
        }
        if let Err(e) = self.sender.send(event) {
            tracing::trace!(error = %e, "ui broadcast: no receivers");
        }
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ServerToUi> {
        self.sender.subscribe()
    }

    pub fn set_import_progress(&self, repo_id: i64, snapshot: ImportProgressSnapshot) {
        if let Ok(mut map) = self.import_progress.write() {
            map.insert(repo_id, snapshot);
        }
    }

    pub fn clear_import_progress(&self, repo_id: i64) {
        if let Ok(mut map) = self.import_progress.write() {
            map.remove(&repo_id);
        }
    }

    #[must_use]
    pub fn current_import_snapshots(&self) -> Vec<(i64, ImportProgressSnapshot)> {
        self.import_progress.read().map_or_else(
            |_| Vec::new(),
            |map| {
                map.iter()
                    .map(|(&repo_id, snap)| (repo_id, snap.clone()))
                    .collect()
            },
        )
    }

    #[must_use]
    pub fn current_active_backups(&self) -> Vec<ActiveBackupSnapshot> {
        self.active_backups
            .read()
            .map_or_else(|_| Vec::new(), |map| map.values().cloned().collect())
    }

    pub fn set_active_backup(&self, snapshot: ActiveBackupSnapshot) {
        if let Ok(mut map) = self.active_backups.write() {
            map.insert(snapshot.repo_id, snapshot);
        }
    }

    pub fn clear_active_backup(&self, repo_id: i64) {
        if let Ok(mut map) = self.active_backups.write() {
            map.remove(&repo_id);
        }
    }

    fn update_active_backup_progress(
        &self,
        repo_id: i64,
        hostname: &str,
        schedule_id: Option<i64>,
        line: &str,
    ) {
        let is_archive_progress =
            serde_json::from_str::<serde_json::Value>(line).is_ok_and(|value| {
                value.get("type").and_then(serde_json::Value::as_str) == Some("archive_progress")
            });
        if !is_archive_progress {
            return;
        }
        if let Ok(mut map) = self.active_backups.write() {
            let entry = map.entry(repo_id).or_insert_with(|| ActiveBackupSnapshot {
                hostname: hostname.to_owned(),
                target_name: String::new(),
                archive_name: None,
                schedule_id,
                repo_id,
                progress_line: None,
            });
            hostname.clone_into(&mut entry.hostname);
            entry.schedule_id = schedule_id;
            entry.progress_line = Some(line.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_progress_events_update_snapshots() {
        let broadcast = UiBroadcast::new();
        broadcast.send(ServerToUi::ImportProgress {
            repo_id: 7,
            progress: 3,
            total: 5,
            message: Some("Finalizing import\u{2026}".to_string()),
        });

        let snapshots = broadcast.current_import_snapshots();
        assert_eq!(snapshots.len(), 1);
        let (repo_id, snapshot) = snapshots.first().unwrap();
        assert_eq!(*repo_id, 7);
        assert_eq!(snapshot.progress, 3);
        assert_eq!(snapshot.total, 5);
        assert_eq!(
            snapshot.message.as_deref(),
            Some("Finalizing import\u{2026}")
        );
    }

    #[test]
    fn backup_events_update_and_clear_active_snapshots() {
        let broadcast = UiBroadcast::new();
        broadcast.set_active_backup(ActiveBackupSnapshot {
            hostname: "web-server-01".to_string(),
            target_name: "server-daily".to_string(),
            archive_name: Some("server-daily-2026-06-27T00:00:00".to_string()),
            schedule_id: Some(8),
            repo_id: 20,
            progress_line: None,
        });
        broadcast.send(ServerToUi::BackupLog {
            hostname: "web-server-01".to_string(),
            schedule_id: Some(8),
            repo_id: 20,
            line: r#"{"type":"archive_progress","nfiles":42,"original_size":1024,"path":"/srv"}"#
                .to_string(),
        });

        let snapshots = broadcast.current_active_backups();
        assert_eq!(snapshots.len(), 1);
        let snapshot = snapshots.first().unwrap();
        assert_eq!(snapshot.repo_id, 20);
        assert_eq!(snapshot.schedule_id, Some(8));
        assert_eq!(
            snapshot.progress_line.as_deref(),
            Some(r#"{"type":"archive_progress","nfiles":42,"original_size":1024,"path":"/srv"}"#)
        );

        broadcast.send(ServerToUi::BackupCompleted {
            hostname: "web-server-01".to_string(),
            target_name: "server-daily".to_string(),
            report: Box::new(shared::types::BackupReport {
                id: shared::types::ReportId(1),
                agent_id: shared::types::AgentId(10),
                repo_id: shared::types::RepoId(20),
                schedule_id: Some(8),
                started_at: chrono::Utc::now(),
                finished_at: chrono::Utc::now(),
                status: shared::types::BackupStatus::Success,
                original_size: 0,
                compressed_size: 0,
                deduplicated_size: 0,
                repo_unique_csize: 0,
                files_processed: 0,
                duration_secs: 0,
                error_message: None,
                warnings: Vec::new(),
                borg_version: None,
                archive_name: None,
                borg_command: None,
                run_id: None,
            }),
        });

        assert!(broadcast.current_active_backups().is_empty());
    }
}
