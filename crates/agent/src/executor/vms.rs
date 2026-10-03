// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use shared::{protocol::AgentToServer, task_registry::TaskRegistry, vm::VmSnapshotConfig};
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::{Executor, send_outbound};
use crate::{
    backup::{BackupEngine, BackupTarget},
    vm::VmStager,
};

/// Stages the domains of a host whose schedule asked for them, if any. A
/// target without virtual-machine settings is not an error, it is a backup
/// that does not carry machines.
pub(super) async fn stage_configured_machines(
    target: &BackupTarget,
    schedule_id: Option<i64>,
    outbound_tx: &mpsc::Sender<AgentToServer>,
    engine: &BackupEngine,
) -> Result<(), String> {
    let Some(vm_config) = target.vm_snapshot.clone() else {
        return Ok(());
    };
    stage_virtual_machines(
        vm_config,
        schedule_id,
        outbound_tx,
        engine.task_registry().clone(),
    )
    .await
}

/// Stages this host's domains and reports what happened to each. Returns the
/// reason the backup must not go ahead, when a domain failed.
pub(super) async fn stage_virtual_machines(
    config: shared::vm::VmSnapshotConfig,
    schedule_id: Option<i64>,
    outbound_tx: &mpsc::Sender<AgentToServer>,
    task_registry: TaskRegistry,
) -> Result<(), String> {
    info!("Staging virtual machines into {}", config.staging_dir);
    let outcomes = match VmStager::new(config, task_registry).stage_all().await {
        Ok(outcomes) => outcomes,
        Err(e) => {
            // Tell the server the phase ran and found nothing, then fail the
            // backup: an archive that silently holds no virtual machines is
            // worse than a run the operator is told about.
            let msg = AgentToServer::VmSnapshotReport {
                schedule_id,
                outcomes: Vec::new(),
            };
            send_outbound(outbound_tx, msg).await;
            return Err(format!(
                "could not list the virtual machines of this host: {e}"
            ));
        }
    };

    let failures: Vec<String> = outcomes
        .iter()
        .filter_map(|outcome| {
            outcome
                .error
                .as_ref()
                .map(|error| format!("{}: {error}", outcome.name))
        })
        .collect();

    let msg = AgentToServer::VmSnapshotReport {
        schedule_id,
        outcomes,
    };
    send_outbound(outbound_tx, msg).await;

    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "could not stage {} virtual machine(s): {}",
            failures.len(),
            failures.join("; ")
        ))
    }
}

impl Executor {
    /// Reports which domains this host has, in answer to a scan request. A
    /// host with staging switched off still answers: the operator is usually
    /// scanning precisely because they are about to switch it on.
    pub(super) async fn handle_scan_vms(
        &self,
        request_id: Option<String>,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        let config = self
            .current_config
            .lock()
            .await
            .as_ref()
            .map_or_else(VmSnapshotConfig::default, |config| {
                config.vm_snapshot.clone()
            });

        let (vms, error) = match VmStager::new(config, self.task_registry.clone())
            .scan()
            .await
        {
            Ok(vms) => (vms, None),
            Err(e) => {
                warn!("Virtual machine scan failed: {e}");
                (Vec::new(), Some(e.to_string()))
            }
        };

        let _ = outbound_tx
            .send(AgentToServer::VmScanResult {
                request_id,
                vms,
                error,
            })
            .await;
    }

    /// Builds a domain out of files a restore put back on disk. The host's
    /// own staging settings do not apply: the request says where to read from
    /// and where the images go.
    pub(super) async fn handle_build_vm(
        &self,
        request_id: String,
        request: &shared::vm::VmBuildRequest,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        info!(
            "Building virtual machine {} from {}",
            request.name, request.source_dir
        );
        let (outcome, error) =
            match VmStager::new(VmSnapshotConfig::default(), self.task_registry.clone())
                .build(request)
                .await
            {
                Ok(outcome) => (Some(outcome), None),
                Err(e) => {
                    warn!("Virtual machine build failed: {e}");
                    (None, Some(e.to_string()))
                }
            };

        let _ = outbound_tx
            .send(AgentToServer::VmBuildResult {
                request_id,
                outcome,
                error,
            })
            .await;
    }

    /// Stages one domain right now, in answer to a manual "snapshot now"
    /// request. Uses the host's own staging settings, the same as a
    /// schedule's backup would, so the domain lands wherever the operator
    /// configured.
    pub(super) async fn handle_stage_vm(
        &self,
        request_id: Option<String>,
        domain: String,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        let config = self
            .current_config
            .lock()
            .await
            .as_ref()
            .map_or_else(VmSnapshotConfig::default, |config| {
                config.vm_snapshot.clone()
            });

        let outcome = VmStager::new(config, self.task_registry.clone())
            .stage_one(&domain)
            .await;

        let _ = outbound_tx
            .send(AgentToServer::VmStageResult {
                request_id,
                outcome,
            })
            .await;
    }
}
