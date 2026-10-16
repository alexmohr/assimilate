// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Endpoints for a host's database dumps: the settings that govern how its
//! agent dumps the databases in its Kubernetes cluster into a backup, the
//! database workloads it found, and the operator's choice for each.

use std::time::Duration;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use shared::{
    database::{DatabaseSelectionMode, DatabaseSourceId, is_valid_dump_dir},
    protocol::ServerToAgent,
    responses::{
        AgentDatabaseSettingsResponse, AgentDatabaseSourceResponse, AgentDatabasesResponse,
    },
};
use tokio::sync::oneshot;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    auth::{AuthUser, RequireAdmin},
    helpers::DomainQuery,
    vms::validate_absolute_dir,
};
use crate::{
    AppState, config_assembler,
    db::{self, databases::AgentDatabaseSourceRow},
    error::{ApiError, ApiJson},
};

/// How long the server waits for a scan. The agent asks every database pod
/// for its databases, one exec at a time.
const SCAN_TIMEOUT: Duration = Duration::from_mins(2);

/// New dump settings for a host.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateAgentDatabaseBackupRequest {
    /// Whether this host dumps databases at all.
    pub enabled: bool,
    /// Absolute directory on the host that holds the dump pipes while a
    /// backup runs. Not below `/dev`, `/proc`, `/sys` or `/run`, which a
    /// dumping run leaves out of the archive.
    pub dump_dir: String,
    /// Kubeconfig file on the host. Empty or absent uses the agent's default:
    /// the in-cluster service account, or `$KUBECONFIG` and `~/.kube/config`.
    #[serde(default)]
    pub kubeconfig_path: Option<String>,
    /// Which sources the per-source flags select. Absent keeps the host's
    /// mode.
    #[serde(default)]
    pub selection: Option<DatabaseSelectionMode>,
}

/// The operator's choice for one source.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateAgentDatabaseSourceRequest {
    /// Whether backups dump this source. `null` hands the choice back to the
    /// host's selection mode.
    pub included: Option<bool>,
}

/// What to scan with.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct ScanAgentDatabasesRequest {
    /// Kubeconfig file to try instead of the saved one, so a path can be
    /// tested before it is saved.
    #[serde(default)]
    pub kubeconfig_path: Option<String>,
}

fn source_to_response(
    row: AgentDatabaseSourceRow,
    selection: DatabaseSelectionMode,
) -> AgentDatabaseSourceResponse {
    AgentDatabaseSourceResponse {
        included: row
            .included
            .unwrap_or_else(|| selection.includes_untouched()),
        namespace: row.namespace,
        name: row.name,
        kind: row.kind,
        engine: row.engine,
        pod: row.pod,
        container: row.container,
        databases: row.databases,
        last_error: row.last_error,
        last_scanned_at: row.last_scanned_at,
    }
}

async fn build_response(
    state: &AppState,
    agent_id: i64,
) -> Result<AgentDatabasesResponse, ApiError> {
    let settings = db::databases::get_agent_database_backup(&state.pool, agent_id).await?;
    let rows = db::databases::list_agent_database_sources(&state.pool, agent_id).await?;
    let selection = settings.database_selection;
    Ok(AgentDatabasesResponse {
        settings: AgentDatabaseSettingsResponse {
            enabled: settings.database_backup_enabled,
            dump_dir: settings.database_dump_dir,
            kubeconfig_path: settings.database_kubeconfig_path,
            selection,
        },
        sources: rows
            .into_iter()
            .map(|row| source_to_response(row, selection))
            .collect(),
    })
}

/// An absolute dump directory outside the trees a dumping run leaves out.
fn validate_dump_dir(dir: &str) -> Result<(), ApiError> {
    validate_absolute_dir("dump directory", dir)?;
    if !is_valid_dump_dir(dir.trim()) {
        return Err(ApiError::BadRequest(
            "the dump directory must not be below /dev, /proc, /sys or /run: a run that dumps \
             databases leaves those out of the archive"
                .to_owned(),
        ));
    }
    Ok(())
}

/// The kubeconfig path to store: `None` for an empty one, otherwise an
/// absolute path.
fn kubeconfig_path(path: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        return Ok(None);
    };
    validate_absolute_dir("kubeconfig path", path)?;
    Ok(Some(path.to_owned()))
}

fn source_id(namespace: &str, name: &str) -> Result<DatabaseSourceId, ApiError> {
    if !db::databases::is_kube_name(namespace) || !db::databases::is_kube_name(name) {
        return Err(ApiError::BadRequest(
            "the namespace and name must be Kubernetes names".to_owned(),
        ));
    }
    Ok(DatabaseSourceId {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
    })
}

#[utoipa::path(
    get,
    path = "/api/agents/{hostname}/databases",
    tag = "Agents",
    operation_id = "getAgentDatabases",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    responses(
        (status = 200, description = "Settings and sources", body = AgentDatabasesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Read a host's database dump settings and the database workloads its agent
/// last found.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] when the agent does not exist.
pub async fn get_agent_databases(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
) -> Result<Json<AgentDatabasesResponse>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;
    Ok(Json(build_response(&state, agent.id).await?))
}

#[utoipa::path(
    put,
    path = "/api/agents/{hostname}/database-backup",
    tag = "Agents",
    operation_id = "updateAgentDatabaseBackup",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    request_body = UpdateAgentDatabaseBackupRequest,
    responses(
        (status = 200, description = "Updated settings", body = AgentDatabasesResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Update a host's database dump settings.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::NotFound`]: the agent does not exist
pub async fn update_agent_database_backup(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
    ApiJson(req): ApiJson<UpdateAgentDatabaseBackupRequest>,
) -> Result<Json<AgentDatabasesResponse>, ApiError> {
    validate_dump_dir(&req.dump_dir)?;
    let kubeconfig_path = kubeconfig_path(req.kubeconfig_path.as_deref())?;
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;

    db::databases::update_agent_database_backup(
        &state.pool,
        agent.id,
        db::databases::DatabaseBackupPatch {
            enabled: req.enabled,
            dump_dir: req.dump_dir.trim(),
            kubeconfig_path: kubeconfig_path.as_deref(),
            selection: req.selection,
        },
    )
    .await?;

    config_assembler::push_config_to_agent(&state, agent.id).await;
    Ok(Json(build_response(&state, agent.id).await?))
}

#[utoipa::path(
    put,
    path = "/api/agents/{hostname}/databases/{namespace}/{name}",
    tag = "Agents",
    operation_id = "updateAgentDatabaseSource",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("namespace" = String, Path, description = "Kubernetes namespace"),
        ("name" = String, Path, description = "Cluster or workload name"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    request_body = UpdateAgentDatabaseSourceRequest,
    responses(
        (status = 200, description = "Updated sources", body = AgentDatabasesResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Choose whether backups dump one database source.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the namespace or name is not a Kubernetes name
/// - [`ApiError::NotFound`]: the agent does not exist
pub async fn update_agent_database_source(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path((hostname, namespace, name)): Path<(String, String, String)>,
    Query(query): Query<DomainQuery>,
    ApiJson(req): ApiJson<UpdateAgentDatabaseSourceRequest>,
) -> Result<Json<AgentDatabasesResponse>, ApiError> {
    let id = source_id(&namespace, &name)?;
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;
    db::databases::set_database_source_included(&state.pool, agent.id, &id, req.included).await?;
    config_assembler::push_config_to_agent(&state, agent.id).await;
    Ok(Json(build_response(&state, agent.id).await?))
}

#[utoipa::path(
    post,
    path = "/api/agents/{hostname}/databases/scan",
    tag = "Agents",
    operation_id = "scanAgentDatabases",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    request_body = ScanAgentDatabasesRequest,
    responses(
        (status = 200, description = "Freshly scanned sources", body = AgentDatabasesResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
        (status = 502, description = "The agent could not reach its cluster"),
        (status = 503, description = "The agent is not connected"),
    )
)]
/// Ask a host's agent which database workloads its cluster runs, and wait for
/// the answer.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the kubeconfig path is not absolute
/// - [`ApiError::NotFound`]: the agent does not exist
/// - [`ApiError::ServiceUnavailable`]: the agent is not connected, or did not
///   answer in time
/// - [`ApiError::BadGateway`]: the agent could not reach its cluster, or is too
///   old to dump databases
pub async fn scan_agent_databases(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
    ApiJson(req): ApiJson<ScanAgentDatabasesRequest>,
) -> Result<Json<AgentDatabasesResponse>, ApiError> {
    let kubeconfig_path = kubeconfig_path(req.kubeconfig_path.as_deref())?;
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;

    let request_id = Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    state
        .pending_database_scans
        .insert(request_id.clone(), agent.id, tx)
        .await;

    if state
        .registry
        .send_to(
            agent.id,
            ServerToAgent::ScanDatabases {
                request_id: request_id.clone(),
                kubeconfig_path,
            },
        )
        .await
        .is_err()
    {
        state.pending_database_scans.remove(&request_id).await;
        return Err(ApiError::ServiceUnavailable(format!(
            "agent '{hostname}' is not connected"
        )));
    }

    wait_for_scan(&state, &hostname, &request_id, rx, SCAN_TIMEOUT).await?;
    Ok(Json(build_response(&state, agent.id).await?))
}

/// Waits up to `timeout` for the agent's answer to scan `request_id`. A
/// request that times out is withdrawn, so a late answer finds nothing to
/// resolve.
async fn wait_for_scan(
    state: &AppState,
    hostname: &str,
    request_id: &str,
    rx: oneshot::Receiver<(
        Vec<shared::database::DiscoveredDatabaseSource>,
        Option<String>,
    )>,
    timeout: Duration,
) -> Result<(), ApiError> {
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok((_, None))) => Ok(()),
        Ok(Ok((_, Some(error)))) => Err(ApiError::BadGateway(format!(
            "agent '{hostname}' could not scan its cluster: {error}"
        ))),
        Ok(Err(_)) => Err(ApiError::ServiceUnavailable(format!(
            "agent '{hostname}' disconnected before answering the scan"
        ))),
        Err(_) => {
            state.pending_database_scans.remove(request_id).await;
            Err(ApiError::ServiceUnavailable(format!(
                "agent '{hostname}' did not answer the scan within {} seconds",
                timeout.as_secs()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use shared::database::{DatabaseEngine, DatabaseSourceKind, DiscoveredDatabaseSource};
    use sqlx::PgPool;
    use tokio::sync::mpsc;

    use super::*;
    use crate::{api::auth::AuthUser, pending::Claim};

    struct ScanFixture {
        state: AppState,
        admin: AuthUser,
        agent_id: i64,
    }

    /// A connected agent, and the channel its server-sent requests arrive on.
    async fn scan_fixture(pool: &PgPool) -> (ScanFixture, mpsc::Receiver<ServerToAgent>) {
        let state = crate::test_support::build_test_state(pool.clone(), b"database-scan-key");
        let admin = crate::test_support::insert_auth_user(pool, "admin").await;
        let agent = db::insert_agent(pool, "db-host", None, "hash", None, None)
            .await
            .unwrap();
        let (tx, requests) = mpsc::channel(4);
        state
            .registry
            .register(agent.id, tx, false, None, None)
            .await;
        let fixture = ScanFixture {
            state,
            admin,
            agent_id: agent.id,
        };
        (fixture, requests)
    }

    async fn scan(fixture: &ScanFixture) -> Result<Json<AgentDatabasesResponse>, ApiError> {
        scan_agent_databases(
            State(fixture.state.clone()),
            RequireAdmin(fixture.admin.clone()),
            Path("db-host".to_owned()),
            Query(DomainQuery { domain: None }),
            ApiJson(ScanAgentDatabasesRequest {
                kubeconfig_path: Some("/etc/assimilate/kubeconfig".to_owned()),
            }),
        )
        .await
    }

    /// Plays the agent: takes the scan request and answers it with what
    /// `answer` returns, or drops it unanswered when that is `None`.
    fn answer_with(
        mut requests: mpsc::Receiver<ServerToAgent>,
        state: AppState,
        agent_id: i64,
        answer: Option<(Vec<DiscoveredDatabaseSource>, Option<String>)>,
    ) {
        tokio::spawn(async move {
            let Some(ServerToAgent::ScanDatabases {
                request_id,
                kubeconfig_path,
            }) = requests.recv().await
            else {
                panic!("the agent is asked to scan");
            };
            assert_eq!(
                kubeconfig_path.as_deref(),
                Some("/etc/assimilate/kubeconfig")
            );
            if let Some((sources, error)) = &answer
                && error.is_none()
            {
                db::databases::record_database_scan(&state.pool, agent_id, sources)
                    .await
                    .unwrap();
            }
            let Claim::Claimed(tx) = state
                .pending_database_scans
                .claim(&request_id, agent_id)
                .await
            else {
                panic!("the scan is pending for this agent");
            };
            if let Some(answer) = answer {
                let _ = tx.send(answer);
            }
        });
    }

    fn source() -> DiscoveredDatabaseSource {
        DiscoveredDatabaseSource {
            id: DatabaseSourceId {
                namespace: "shop".to_owned(),
                name: "orders-db".to_owned(),
            },
            kind: DatabaseSourceKind::CnpgCluster,
            engine: DatabaseEngine::Postgresql,
            pod: "orders-db-2".to_owned(),
            container: "postgres".to_owned(),
            databases: vec!["orders".to_owned()],
            error: None,
        }
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_scan_the_agent_answers_returns_the_fresh_sources(pool: PgPool) {
        let (fixture, requests) = scan_fixture(&pool).await;
        answer_with(
            requests,
            fixture.state.clone(),
            fixture.agent_id,
            Some((vec![source()], None)),
        );

        let Json(response) = scan(&fixture).await.unwrap();

        let found = response.sources.first().unwrap();
        assert_eq!(found.name, "orders-db");
        assert_eq!(found.databases, ["orders"]);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_scan_the_agent_fails_says_why(pool: PgPool) {
        let (fixture, requests) = scan_fixture(&pool).await;
        answer_with(
            requests,
            fixture.state.clone(),
            fixture.agent_id,
            Some((Vec::new(), Some("pods is forbidden".to_owned()))),
        );

        let Err(ApiError::BadGateway(message)) = scan(&fixture).await else {
            panic!("a failed scan is a bad gateway");
        };
        assert!(message.contains("pods is forbidden"), "{message}");
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_scan_dropped_by_a_disconnecting_agent_is_unavailable(pool: PgPool) {
        let (fixture, requests) = scan_fixture(&pool).await;
        answer_with(requests, fixture.state.clone(), fixture.agent_id, None);

        let Err(ApiError::ServiceUnavailable(message)) = scan(&fixture).await else {
            panic!("an unanswered scan is unavailable");
        };
        assert!(message.contains("disconnected"), "{message}");
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_scan_nobody_answers_is_withdrawn_when_it_times_out(pool: PgPool) {
        let (fixture, _requests) = scan_fixture(&pool).await;
        let (tx, rx) = oneshot::channel();
        fixture
            .state
            .pending_database_scans
            .insert("req-late".to_owned(), fixture.agent_id, tx)
            .await;

        let result = wait_for_scan(
            &fixture.state,
            "db-host",
            "req-late",
            rx,
            Duration::from_millis(10),
        )
        .await;

        let Err(ApiError::ServiceUnavailable(message)) = result else {
            panic!("a scan nobody answers is unavailable");
        };
        assert!(message.contains("did not answer"), "{message}");
        assert!(matches!(
            fixture
                .state
                .pending_database_scans
                .claim("req-late", fixture.agent_id)
                .await,
            Claim::Unknown
        ));
    }

    #[test]
    fn dump_dirs_must_be_absolute_and_outside_the_excluded_trees() {
        assert!(validate_dump_dir("/var/lib/assimilate/database-dumps").is_ok());
        assert!(validate_dump_dir(" /srv/dumps ").is_ok());
        assert!(validate_dump_dir("relative").is_err());
        assert!(validate_dump_dir("/").is_err());
        assert!(validate_dump_dir("/run/dumps").is_err());
        assert!(validate_dump_dir("/proc").is_err());
        assert!(validate_dump_dir("/srv/../run").is_err());
    }

    #[test]
    fn an_empty_kubeconfig_path_means_the_default() {
        assert_eq!(kubeconfig_path(None).unwrap(), None);
        assert_eq!(kubeconfig_path(Some("  ")).unwrap(), None);
        assert_eq!(
            kubeconfig_path(Some(" /etc/assimilate/kubeconfig ")).unwrap(),
            Some("/etc/assimilate/kubeconfig".to_owned())
        );
        assert!(kubeconfig_path(Some("kubeconfig")).is_err());
        assert!(kubeconfig_path(Some("/etc/../kubeconfig")).is_err());
    }

    #[test]
    fn source_ids_are_kubernetes_names() {
        assert!(source_id("apps", "pg").is_ok());
        assert!(source_id("apps", "../x").is_err());
        assert!(source_id("Apps", "pg").is_err());
    }

    #[test]
    fn an_undecided_source_follows_the_selection_mode() {
        let row = AgentDatabaseSourceRow {
            namespace: "apps".to_owned(),
            name: "pg".to_owned(),
            included: None,
            kind: None,
            engine: None,
            pod: None,
            container: None,
            databases: Vec::new(),
            last_error: None,
            last_scanned_at: None,
        };
        assert!(source_to_response(row.clone(), DatabaseSelectionMode::All).included);
        assert!(!source_to_response(row.clone(), DatabaseSelectionMode::Selected).included);
        let chosen = AgentDatabaseSourceRow {
            included: Some(true),
            ..row
        };
        assert!(source_to_response(chosen, DatabaseSelectionMode::Selected).included);
    }
}
