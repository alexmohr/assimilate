// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use shared::types::{AgentConfig, BorgEncryption, RepoId};

pub enum ExecutorCommand {
    UpdateConfig(Box<AgentConfig>),
    RunNow {
        repo_id: RepoId,
        schedule_id: Option<i64>,
        run_id: Option<String>,
    },
    CancelBackup {
        repo_id: RepoId,
    },
    ScanVms {
        request_id: Option<String>,
    },
    ScanDatabases(ScanDatabasesRequest),
    BuildVm {
        request_id: String,
        request: shared::vm::VmBuildRequest,
    },
    StageVm {
        request_id: Option<String>,
        domain: String,
    },
    RunCheckNow {
        repo_id: RepoId,
    },
    RunVerifyNow {
        repo_id: RepoId,
    },
    InitRepo {
        repo_path: String,
        ssh_user: String,
        ssh_host: String,
        ssh_port: u16,
        passphrase: String,
        encryption: BorgEncryption,
    },
    DryRun {
        repo_id: RepoId,
        schedule_id: i64,
        request_id: String,
    },
    RestoreFiles {
        repo_id: RepoId,
        archive_name: String,
        paths: Vec<String>,
        target_path: String,
        request_id: String,
    },
    RestoreDatabase(RestoreDatabaseRequest),
    DeleteArchives {
        repo_id: RepoId,
        archive_names: Vec<String>,
        request_id: String,
    },
    ReadLiveFile(LiveFileRequest),
}

/// A request to read one file as it is on the host now.
pub struct LiveFileRequest {
    pub request_id: String,
    /// The file's path as an archive holds it.
    pub path: String,
}

/// A request to list the database workloads on the host.
pub struct ScanDatabasesRequest {
    pub request_id: String,
    /// Runtimes to search instead of the host's own settings.
    pub runtimes: Option<Box<shared::database::DatabaseRuntimes>>,
}

/// A request to restore a database dump from an archive into the database
/// server its source runs on now.
pub struct RestoreDatabaseRequest {
    pub request_id: String,
    pub repo_id: RepoId,
    pub archive_name: String,
    pub restore: shared::database::DatabaseRestore,
}
