// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use shared::types::{AgentConfig, BorgEncryption, RepoId};

pub enum ExecutorCommand {
    UpdateConfig(AgentConfig),
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
    DeleteArchives {
        repo_id: RepoId,
        archive_names: Vec<String>,
        request_id: String,
    },
}
