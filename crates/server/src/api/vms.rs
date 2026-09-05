// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Endpoints for a host's virtual-machine staging: the settings that govern
//! how its agent stages libvirt/QEMU domains before a backup, the domains it
//! reported, and the per-domain settings the operator makes.

use std::{str::FromStr, time::Duration};

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use shared::{
    protocol::ServerToAgent,
    responses::{AgentVmResponse, AgentVmSnapshotResponse, AgentVmSnapshotSettingsResponse},
    vm::{VmBuildAction, VmBuildOutcome, VmBuildRequest, VmSelectionMode, VmSnapshotMode, VmState},
};
use tokio::sync::oneshot;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    auth::{AuthUser, RequireAdmin},
    helpers::DomainQuery,
};
use crate::{
    AppState, config_assembler,
    db::{self, vms::AgentVmRow},
    error::{ApiError, ApiJson},
};

/// How long the server waits for an agent to answer a scan request before
/// giving up. Enumerating domains is a handful of local commands, so a host
/// that has not answered by now is not going to.
const SCAN_TIMEOUT: Duration = Duration::from_secs(60);

/// How long the server waits for a build. Merging a chain is disk-bound and
/// runs at the speed of the target host's storage, so this is generous.
const BUILD_TIMEOUT: Duration = Duration::from_hours(4);

/// Largest staging directory depth we accept, to keep a typo from pointing the
/// staging directory at a filesystem root.
const MIN_STAGING_DIR_LEN: usize = 2;

/// New staging settings for a host.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateAgentVmSnapshotRequest {
    /// Whether this host stages its domains at all.
    pub enabled: bool,
    /// Absolute directory receiving one subdirectory per domain.
    pub staging_dir: String,
    /// Increments written before a new full image is taken.
    pub full_interval: u32,
    /// Seconds one domain's snapshot may take.
    pub timeout_seconds: u32,
    /// Bytes a domain may occupy unless it carries its own limit. Zero means
    /// no limit.
    pub default_limit_bytes: u64,
    /// Which domains the per-domain flags select. Absent keeps whichever mode
    /// the host already had, so a client that predates the setting cannot
    /// silently flip a host to staging nothing.
    #[serde(default)]
    pub selection: Option<VmSelectionMode>,
}

/// New settings for one domain of a host.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateAgentVmRequest {
    /// Whether the domain is staged at all. Absent keeps whichever decision
    /// the domain already carried, so an edit that only changes the limit
    /// cannot also opt an undecided domain in.
    #[serde(default)]
    pub included: Option<bool>,
    /// Bytes this domain may occupy, or `null` to inherit the host's default.
    pub limit_bytes: Option<u64>,
}

/// Renders one stored domain row, resolving the limit that actually applies,
/// resolving a domain nobody has decided about against the host's selection
/// mode, and reporting an excluded domain as excluded whatever the last scan
/// saw.
fn vm_to_response(
    row: AgentVmRow,
    default_limit_bytes: u64,
    selection: VmSelectionMode,
) -> AgentVmResponse {
    let limit_bytes = row
        .limit_bytes
        .map(|limit| u64::try_from(limit).unwrap_or(0));
    // The client is told what will actually happen, not what is stored: an
    // undecided domain reads as included under `all` and excluded under
    // `selected`, which is what the agent will do with it.
    let included = row
        .included
        .unwrap_or_else(|| selection.includes_untouched());
    let mode = if included {
        VmSnapshotMode::from_str(&row.mode).unwrap_or_default()
    } else {
        VmSnapshotMode::Excluded
    };

    AgentVmResponse {
        name: row.name,
        included,
        limit_bytes,
        effective_limit_bytes: limit_bytes.unwrap_or(default_limit_bytes),
        state: VmState::from_str(&row.state).unwrap_or_default(),
        mode,
        disk_count: u32::try_from(row.disk_count).unwrap_or(0),
        disk_bytes: u64::try_from(row.disk_bytes).unwrap_or(0),
        staged_bytes: u64::try_from(row.staged_bytes).unwrap_or(0),
        chain_length: u32::try_from(row.chain_length).unwrap_or(0),
        last_error: row.last_error,
        last_scanned_at: row.last_scanned_at,
        last_staged_at: row.last_staged_at,
    }
}

/// Reads a host's settings and domains and renders them together.
async fn build_response(
    state: &AppState,
    agent_id: i64,
) -> Result<AgentVmSnapshotResponse, ApiError> {
    let settings = db::vms::get_agent_vm_snapshot(&state.pool, agent_id).await?;
    let rows = db::vms::list_agent_vms(&state.pool, agent_id).await?;
    let default_limit_bytes = u64::try_from(settings.vm_snapshot_default_limit_bytes).unwrap_or(0);
    let selection = VmSelectionMode::from_str(&settings.vm_snapshot_selection).unwrap_or_default();

    Ok(AgentVmSnapshotResponse {
        settings: AgentVmSnapshotSettingsResponse {
            enabled: settings.vm_snapshot_enabled,
            staging_dir: settings.vm_snapshot_dir,
            full_interval: u32::try_from(settings.vm_snapshot_full_interval).unwrap_or(1),
            timeout_seconds: u32::try_from(settings.vm_snapshot_timeout_seconds).unwrap_or(1),
            default_limit_bytes,
            selection,
        },
        vms: rows
            .into_iter()
            .map(|row| vm_to_response(row, default_limit_bytes, selection))
            .collect(),
    })
}

/// Rejects a staging directory that would send the agent somewhere it cannot
/// reason about: a relative path resolves against the agent's working
/// directory, and a filesystem root would put every domain beside the system's
/// own directories.
fn validate_staging_dir(dir: &str) -> Result<(), ApiError> {
    let trimmed = dir.trim();
    if !trimmed.starts_with('/') {
        return Err(ApiError::BadRequest(
            "the staging directory must be an absolute path".to_owned(),
        ));
    }
    if trimmed.len() < MIN_STAGING_DIR_LEN {
        return Err(ApiError::BadRequest(
            "the staging directory must not be the filesystem root".to_owned(),
        ));
    }
    if trimmed.contains("..") {
        return Err(ApiError::BadRequest(
            "the staging directory must not contain '..'".to_owned(),
        ));
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/api/agents/{hostname}/vms",
    tag = "Agents",
    operation_id = "getAgentVms",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    responses(
        (status = 200, description = "Settings and domains", body = AgentVmSnapshotResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Read a host's virtual-machine staging settings and the domains its agent
/// last reported.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] when the agent does not exist.
pub async fn get_agent_vms(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
) -> Result<Json<AgentVmSnapshotResponse>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;
    Ok(Json(build_response(&state, agent.id).await?))
}

#[utoipa::path(
    put,
    path = "/api/agents/{hostname}/vm-snapshot",
    tag = "Agents",
    operation_id = "updateAgentVmSnapshot",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    request_body = UpdateAgentVmSnapshotRequest,
    responses(
        (status = 200, description = "Updated settings", body = AgentVmSnapshotResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Update a host's virtual-machine staging settings.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::NotFound`]: the agent does not exist
pub async fn update_agent_vm_snapshot(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
    ApiJson(req): ApiJson<UpdateAgentVmSnapshotRequest>,
) -> Result<Json<AgentVmSnapshotResponse>, ApiError> {
    validate_staging_dir(&req.staging_dir)?;
    if req.full_interval == 0 {
        return Err(ApiError::BadRequest(
            "a full image must be written at least every increment".to_owned(),
        ));
    }
    if req.timeout_seconds == 0 {
        return Err(ApiError::BadRequest(
            "the snapshot timeout must be greater than zero".to_owned(),
        ));
    }

    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;

    db::vms::update_agent_vm_snapshot(
        &state.pool,
        agent.id,
        db::vms::VmSnapshotPatch {
            enabled: req.enabled,
            dir: req.staging_dir.trim(),
            full_interval: i32::try_from(req.full_interval)
                .map_err(|_| ApiError::BadRequest("full_interval out of range".to_owned()))?,
            timeout_seconds: i32::try_from(req.timeout_seconds)
                .map_err(|_| ApiError::BadRequest("timeout_seconds out of range".to_owned()))?,
            default_limit_bytes: i64::try_from(req.default_limit_bytes)
                .map_err(|_| ApiError::BadRequest("default_limit_bytes out of range".to_owned()))?,
            selection: match req.selection {
                Some(selection) => selection,
                None => VmSelectionMode::from_str(
                    &db::vms::get_agent_vm_snapshot(&state.pool, agent.id)
                        .await?
                        .vm_snapshot_selection,
                )
                .unwrap_or_default(),
            },
        },
    )
    .await?;

    config_assembler::push_config_to_agent(&state, agent.id).await;
    Ok(Json(build_response(&state, agent.id).await?))
}

#[utoipa::path(
    put,
    path = "/api/agents/{hostname}/vms/{name}",
    tag = "Agents",
    operation_id = "updateAgentVm",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("name" = String, Path, description = "libvirt domain name"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    request_body = UpdateAgentVmRequest,
    responses(
        (status = 200, description = "Updated settings", body = AgentVmSnapshotResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Update one domain's staging settings.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::NotFound`]: the agent does not exist
pub async fn update_agent_vm(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path((hostname, name)): Path<(String, String)>,
    Query(query): Query<DomainQuery>,
    ApiJson(req): ApiJson<UpdateAgentVmRequest>,
) -> Result<Json<AgentVmSnapshotResponse>, ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::BadRequest("a domain name is required".to_owned()));
    }

    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;
    let limit_bytes = req
        .limit_bytes
        .map(|limit| {
            i64::try_from(limit)
                .map_err(|_| ApiError::BadRequest("limit_bytes out of range".to_owned()))
        })
        .transpose()?;

    db::vms::set_vm_settings(
        &state.pool,
        agent.id,
        name.trim(),
        req.included,
        limit_bytes,
    )
    .await?;

    config_assembler::push_config_to_agent(&state, agent.id).await;
    Ok(Json(build_response(&state, agent.id).await?))
}

#[utoipa::path(
    post,
    path = "/api/agents/{hostname}/vms/scan",
    tag = "Agents",
    operation_id = "scanAgentVms",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    responses(
        (status = 200, description = "Freshly scanned domains", body = AgentVmSnapshotResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
        (status = 502, description = "The agent could not scan its host"),
        (status = 503, description = "The agent is not connected"),
    )
)]
/// Ask a host's agent which domains it has, and wait for the answer.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the agent does not exist
/// - [`ApiError::ServiceUnavailable`]: the agent is not connected, or did not
///   answer in time
/// - [`ApiError::BadGateway`]: the agent could not scan its host
pub async fn scan_agent_vms(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
) -> Result<Json<AgentVmSnapshotResponse>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;

    let request_id = Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    state
        .pending_vm_scans
        .lock()
        .await
        .insert(request_id.clone(), tx);

    if state
        .registry
        .send_to(
            agent.id,
            ServerToAgent::ScanVms {
                request_id: Some(request_id.clone()),
            },
        )
        .await
        .is_err()
    {
        state.pending_vm_scans.lock().await.remove(&request_id);
        return Err(ApiError::ServiceUnavailable(format!(
            "agent '{hostname}' is not connected"
        )));
    }

    match tokio::time::timeout(SCAN_TIMEOUT, rx).await {
        Ok(Ok((_, Some(error)))) => Err(ApiError::BadGateway(format!(
            "agent '{hostname}' could not scan its host: {error}"
        ))),
        Ok(Ok((_, None))) => Ok(Json(build_response(&state, agent.id).await?)),
        Ok(Err(_)) => Err(ApiError::ServiceUnavailable(format!(
            "agent '{hostname}' disconnected before answering the scan"
        ))),
        Err(_) => {
            state.pending_vm_scans.lock().await.remove(&request_id);
            Err(ApiError::ServiceUnavailable(format!(
                "agent '{hostname}' did not answer the scan within {} seconds",
                SCAN_TIMEOUT.as_secs()
            )))
        }
    }
}

/// What to build, where from, and what to do with it.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct BuildAgentVmRequest {
    /// Directory on the target host holding the restored domain.
    pub source_dir: String,
    /// Name to define the restored domain under.
    pub name: String,
    /// Directory the merged images are moved to.
    pub image_dir: String,
    /// What to do once the images are in place.
    pub action: VmBuildAction,
}

/// Rejects a domain name libvirt would not take, and one that would let a
/// path escape the directory it is written into.
fn validate_domain_name(name: &str) -> Result<(), ApiError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ApiError::BadRequest(
            "a name for the restored domain is required".to_owned(),
        ));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(ApiError::BadRequest(
            "a domain name may only hold letters, digits, '-', '_' and '.'".to_owned(),
        ));
    }
    if trimmed.starts_with('.') {
        return Err(ApiError::BadRequest(
            "a domain name must not start with '.'".to_owned(),
        ));
    }
    Ok(())
}

/// Rejects a path the agent could not act on, or that walks out of itself.
fn validate_absolute_dir(label: &str, path: &str) -> Result<(), ApiError> {
    let trimmed = path.trim();
    if !trimmed.starts_with('/') {
        return Err(ApiError::BadRequest(format!(
            "the {label} must be an absolute path"
        )));
    }
    if trimmed.len() < MIN_STAGING_DIR_LEN {
        return Err(ApiError::BadRequest(format!(
            "the {label} must not be the filesystem root"
        )));
    }
    if trimmed.contains("..") {
        return Err(ApiError::BadRequest(format!(
            "the {label} must not contain '..'"
        )));
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/api/agents/{hostname}/vms/build",
    tag = "Agents",
    operation_id = "buildAgentVm",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    request_body = BuildAgentVmRequest,
    responses(
        (status = 200, description = "The domain that was built", body = VmBuildOutcome),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
        (status = 502, description = "The agent could not build the domain"),
        (status = 503, description = "The agent is not connected"),
    )
)]
/// Build a domain out of files a restore put back on disk: merge the chain,
/// place the images, and define the domain under a new name.
///
/// This is the second stage of a virtual-machine restore. It reads whatever
/// directory it is pointed at, so it also works on files restored earlier.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::NotFound`]: the agent does not exist
/// - [`ApiError::ServiceUnavailable`]: the agent is not connected, or did not
///   answer in time
/// - [`ApiError::BadGateway`]: the agent could not build the domain
pub async fn build_agent_vm(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
    ApiJson(req): ApiJson<BuildAgentVmRequest>,
) -> Result<Json<VmBuildOutcome>, ApiError> {
    validate_domain_name(&req.name)?;
    validate_absolute_dir("source directory", &req.source_dir)?;
    validate_absolute_dir("image directory", &req.image_dir)?;

    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;

    let request_id = Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    state
        .pending_vm_builds
        .lock()
        .await
        .insert(request_id.clone(), tx);

    if state
        .registry
        .send_to(
            agent.id,
            ServerToAgent::BuildVm {
                request_id: request_id.clone(),
                request: VmBuildRequest {
                    source_dir: req.source_dir.trim().to_owned(),
                    name: req.name.trim().to_owned(),
                    image_dir: req.image_dir.trim().to_owned(),
                    action: req.action,
                },
            },
        )
        .await
        .is_err()
    {
        state.pending_vm_builds.lock().await.remove(&request_id);
        return Err(ApiError::ServiceUnavailable(format!(
            "agent '{hostname}' is not connected"
        )));
    }

    match tokio::time::timeout(BUILD_TIMEOUT, rx).await {
        Ok(Ok((Some(outcome), _))) => Ok(Json(outcome)),
        Ok(Ok((None, error))) => Err(ApiError::BadGateway(format!(
            "agent '{hostname}' could not build the domain: {}",
            error.unwrap_or_else(|| "no reason given".to_owned())
        ))),
        Ok(Err(_)) => Err(ApiError::ServiceUnavailable(format!(
            "agent '{hostname}' disconnected before the build finished"
        ))),
        Err(_) => {
            state.pending_vm_builds.lock().await.remove(&request_id);
            Err(ApiError::ServiceUnavailable(format!(
                "agent '{hostname}' did not finish the build within {} minutes",
                BUILD_TIMEOUT.as_secs().saturating_div(60)
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(included: Option<bool>, limit_bytes: Option<i64>) -> AgentVmRow {
        AgentVmRow {
            id: 1,
            name: "web01".to_owned(),
            included,
            limit_bytes,
            state: "running".to_owned(),
            mode: "incremental".to_owned(),
            disk_count: 1,
            disk_bytes: 42,
            staged_bytes: 21,
            chain_length: 4,
            last_error: None,
            last_scanned_at: None,
            last_staged_at: None,
        }
    }

    #[test]
    fn a_domain_without_its_own_limit_inherits_the_host_default() {
        let response = vm_to_response(row(Some(true), None), 200, VmSelectionMode::All);
        assert_eq!(response.limit_bytes, None);
        assert_eq!(response.effective_limit_bytes, 200);
    }

    #[test]
    fn a_domain_with_its_own_limit_keeps_it() {
        let response = vm_to_response(row(Some(true), Some(500)), 200, VmSelectionMode::All);
        assert_eq!(response.limit_bytes, Some(500));
        assert_eq!(response.effective_limit_bytes, 500);
    }

    #[test]
    fn an_excluded_domain_reports_as_excluded_whatever_the_scan_saw() {
        let response = vm_to_response(row(Some(false), None), 200, VmSelectionMode::All);
        assert_eq!(response.mode, VmSnapshotMode::Excluded);
        assert_eq!(response.state, VmState::Running);
    }

    #[test]
    fn an_undecided_domain_follows_the_host_selection_mode() {
        let staged = vm_to_response(row(None, None), 200, VmSelectionMode::All);
        assert!(staged.included);
        assert_eq!(staged.mode, VmSnapshotMode::Incremental);

        let left_alone = vm_to_response(row(None, None), 200, VmSelectionMode::Selected);
        assert!(!left_alone.included);
        assert_eq!(
            left_alone.mode,
            VmSnapshotMode::Excluded,
            "the client is told what will happen, not what the last scan saw"
        );
    }

    #[test]
    fn a_decided_domain_ignores_the_host_selection_mode() {
        for selection in [VmSelectionMode::All, VmSelectionMode::Selected] {
            assert!(vm_to_response(row(Some(true), None), 200, selection).included);
            assert!(!vm_to_response(row(Some(false), None), 200, selection).included);
        }
    }

    /// The limit editor sends no `included` at all. Deserialization has to
    /// keep that distinct from `false`, otherwise a budget edit would read as
    /// an explicit opt-out rather than leaving the decision untouched.
    #[test]
    fn an_absent_included_flag_is_not_a_decision() {
        let limit_only: UpdateAgentVmRequest =
            serde_json::from_str(r#"{"limit_bytes":4096}"#).unwrap();
        assert_eq!(limit_only.included, None);
        assert_eq!(limit_only.limit_bytes, Some(4096));

        for (body, expected) in [
            (r#"{"included":true,"limit_bytes":null}"#, Some(true)),
            (r#"{"included":false,"limit_bytes":null}"#, Some(false)),
        ] {
            let decided: UpdateAgentVmRequest = serde_json::from_str(body).unwrap();
            assert_eq!(decided.included, expected, "an explicit flag is a decision");
        }
    }

    #[test]
    fn a_restored_domain_name_must_be_one_libvirt_takes() {
        assert!(validate_domain_name("web01-restored").is_ok());
        assert!(validate_domain_name("web_01.test").is_ok());
        assert!(validate_domain_name("").is_err());
        assert!(validate_domain_name("   ").is_err());
        assert!(validate_domain_name("web01 restored").is_err());
        assert!(validate_domain_name("../etc/passwd").is_err());
        assert!(validate_domain_name(".hidden").is_err());
        assert!(validate_domain_name("web01;reboot").is_err());
    }

    #[test]
    fn build_directories_must_be_absolute_and_sane() {
        assert!(validate_absolute_dir("source directory", "/var/tmp/restore").is_ok());
        assert!(validate_absolute_dir("image directory", "var/lib/images").is_err());
        assert!(validate_absolute_dir("image directory", "/").is_err());
        assert!(validate_absolute_dir("source directory", "/var/../etc").is_err());
    }

    #[test]
    fn staging_directories_must_be_absolute_and_sane() {
        assert!(validate_staging_dir("/home/virt/backups").is_ok());
        assert!(validate_staging_dir("/srv/vm-staging").is_ok());
        assert!(validate_staging_dir("virt/backups").is_err());
        assert!(validate_staging_dir("/").is_err());
        assert!(validate_staging_dir("/home/../etc").is_err());
    }
}
