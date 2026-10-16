// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Endpoints for a host's database dumps: the settings that govern where its
//! agent looks for databases (its Kubernetes cluster, Docker or Podman
//! containers, the host itself) and how it dumps them into a backup, the
//! database workloads it found, and the operator's choice for each.

use std::time::Duration;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use shared::{
    database::{
        DatabaseRuntime, DatabaseRuntimes, DatabaseSelectionMode, DatabaseSourceId,
        is_valid_dump_dir,
    },
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

/// How long the server waits for a scan. The agent asks every database
/// server for its databases, one at a time.
const SCAN_TIMEOUT: Duration = Duration::from_mins(2);

/// Longest OS user name accepted, as `useradd` caps it.
const MAX_OS_USER_LEN: usize = 32;

/// New dump settings for a host.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateAgentDatabaseBackupRequest {
    /// Whether this host dumps databases at all.
    pub enabled: bool,
    /// Absolute directory on the host that holds the dump pipes while a
    /// backup runs. Not below `/dev`, `/proc`, `/sys` or `/run`, which a
    /// dumping run leaves out of the archive.
    pub dump_dir: String,
    /// Where the host looks for databases. Empty paths and user names mean
    /// the agent's defaults.
    #[serde(default)]
    pub runtimes: DatabaseRuntimes,
    /// Which sources the per-source flags select. Absent keeps the host's
    /// mode.
    #[serde(default)]
    pub selection: Option<DatabaseSelectionMode>,
}

/// The operator's choice for one source.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateAgentDatabaseSourceRequest {
    /// Where the source runs.
    pub runtime: DatabaseRuntime,
    /// The Kubernetes namespace or Compose project, empty when there is none.
    #[serde(default)]
    pub namespace: String,
    /// The cluster, workload, container or Compose service, or `postgresql`
    /// for the host's own server.
    pub name: String,
    /// Whether backups dump this source. `null` hands the choice back to the
    /// host's selection mode.
    pub included: Option<bool>,
}

/// What to scan with.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct ScanAgentDatabasesRequest {
    /// Runtimes to search instead of the saved ones, so settings can be
    /// tried before they are saved.
    #[serde(default)]
    pub runtimes: Option<DatabaseRuntimes>,
}

fn source_to_response(
    row: AgentDatabaseSourceRow,
    selection: DatabaseSelectionMode,
) -> AgentDatabaseSourceResponse {
    AgentDatabaseSourceResponse {
        included: row
            .included
            .unwrap_or_else(|| selection.includes_untouched()),
        runtime: row.runtime,
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
    let selection = settings.selection;
    Ok(AgentDatabasesResponse {
        settings: AgentDatabaseSettingsResponse {
            enabled: settings.enabled,
            dump_dir: settings.dump_dir,
            runtimes: settings.runtimes,
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

/// `value` trimmed, or `None` when nothing is left of it.
fn filled(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// An optional path to store: `None` for an empty one, otherwise an absolute
/// path.
fn optional_path(label: &str, path: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(path) = filled(path) else {
        return Ok(None);
    };
    validate_absolute_dir(label, &path)?;
    Ok(Some(path))
}

/// Whether `name` is a POSIX user name `useradd` would accept.
fn is_os_user_name(name: &str) -> bool {
    name.len() <= MAX_OS_USER_LEN
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
}

/// The runtime settings to store or scan with: text trimmed, empty values
/// dropped for the agent's defaults, and each value checked.
fn validated_runtimes(requested: DatabaseRuntimes) -> Result<DatabaseRuntimes, ApiError> {
    let host_os_user = filled(requested.host_os_user);
    if host_os_user
        .as_deref()
        .is_some_and(|user| !is_os_user_name(user))
    {
        return Err(ApiError::BadRequest(
            "the OS user must be a user name: lower-case letters, digits, '_' and '-'".to_owned(),
        ));
    }
    if requested.host_port == Some(0) {
        return Err(ApiError::BadRequest(
            "the port must be between 1 and 65535".to_owned(),
        ));
    }
    Ok(DatabaseRuntimes {
        kubernetes: requested.kubernetes,
        kubeconfig_path: optional_path("kubeconfig path", requested.kubeconfig_path)?,
        docker: requested.docker,
        docker_socket: optional_path("Docker socket", requested.docker_socket)?,
        host: requested.host,
        host_os_user,
        host_socket_dir: optional_path("socket directory", requested.host_socket_dir)?,
        host_port: requested.host_port,
        host_bin_dir: optional_path("programs directory", requested.host_bin_dir)?,
    })
}

fn source_id(request: &UpdateAgentDatabaseSourceRequest) -> Result<DatabaseSourceId, ApiError> {
    let id = DatabaseSourceId {
        runtime: request.runtime,
        namespace: request.namespace.clone(),
        name: request.name.clone(),
    };
    if !db::databases::is_valid_source_id(&id) {
        return Err(ApiError::BadRequest(format!(
            "'{id}' does not name a {} database source",
            request.runtime
        )));
    }
    Ok(id)
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
    let runtimes = validated_runtimes(req.runtimes)?;
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;

    db::databases::update_agent_database_backup(
        &state.pool,
        agent.id,
        db::databases::DatabaseBackupPatch {
            enabled: req.enabled,
            dump_dir: req.dump_dir.trim(),
            runtimes: &runtimes,
            selection: req.selection,
        },
    )
    .await?;

    config_assembler::push_config_to_agent(&state, agent.id).await;
    Ok(Json(build_response(&state, agent.id).await?))
}

#[utoipa::path(
    put,
    path = "/api/agents/{hostname}/databases/sources",
    tag = "Agents",
    operation_id = "updateAgentDatabaseSource",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
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
/// - [`ApiError::BadRequest`]: the source's names do not fit its runtime
/// - [`ApiError::NotFound`]: the agent does not exist
pub async fn update_agent_database_source(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
    ApiJson(req): ApiJson<UpdateAgentDatabaseSourceRequest>,
) -> Result<Json<AgentDatabasesResponse>, ApiError> {
    let id = source_id(&req)?;
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
        (status = 502, description = "The agent could not search a runtime"),
        (status = 503, description = "The agent is not connected"),
    )
)]
/// Ask a host's agent which database workloads it runs, and wait for the
/// answer.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: a runtime setting to try is invalid
/// - [`ApiError::NotFound`]: the agent does not exist
/// - [`ApiError::ServiceUnavailable`]: the agent is not connected, or did not
///   answer in time
/// - [`ApiError::BadGateway`]: the agent could not search a runtime, or is too
///   old to dump databases
pub async fn scan_agent_databases(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
    ApiJson(req): ApiJson<ScanAgentDatabasesRequest>,
) -> Result<Json<AgentDatabasesResponse>, ApiError> {
    let runtimes = req.runtimes.map(validated_runtimes).transpose()?;
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
                runtimes: runtimes.map(Box::new),
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
            "agent '{hostname}' could not search for databases: {error}"
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
    mod fake_agent;

    use shared::database::{DatabaseEngine, DatabaseSourceKind, DiscoveredDatabaseSource};
    use sqlx::PgPool;
    use tokio::sync::mpsc;

    use self::fake_agent::answer_with;
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
                runtimes: Some(DatabaseRuntimes {
                    kubernetes: true,
                    kubeconfig_path: Some(" /etc/assimilate/kubeconfig ".to_owned()),
                    ..DatabaseRuntimes::default()
                }),
            }),
        )
        .await
    }

    fn source() -> DiscoveredDatabaseSource {
        DiscoveredDatabaseSource {
            id: DatabaseSourceId {
                runtime: DatabaseRuntime::Kubernetes,
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
    fn empty_runtime_settings_mean_the_agents_defaults() {
        let blank = || Some("  ".to_owned());
        let runtimes = validated_runtimes(DatabaseRuntimes {
            kubernetes: true,
            kubeconfig_path: blank(),
            docker: true,
            docker_socket: Some(" /run/podman/podman.sock ".to_owned()),
            host: true,
            host_os_user: blank(),
            host_socket_dir: blank(),
            host_port: Some(5433),
            host_bin_dir: Some("/usr/lib/postgresql/16/bin".to_owned()),
        })
        .unwrap();
        assert_eq!(runtimes.kubeconfig_path, None);
        assert_eq!(
            runtimes.docker_socket.as_deref(),
            Some("/run/podman/podman.sock")
        );
        assert_eq!(runtimes.host_os_user, None);
        assert_eq!(runtimes.host_socket_dir, None);
        assert_eq!(runtimes.host_port, Some(5433));
        assert!(runtimes.kubernetes && runtimes.docker && runtimes.host);
    }

    #[test]
    fn runtime_settings_that_cannot_work_are_refused() {
        let with = |change: fn(&mut DatabaseRuntimes)| {
            let mut runtimes = DatabaseRuntimes::default();
            change(&mut runtimes);
            validated_runtimes(runtimes)
        };
        let refused: [fn(&mut DatabaseRuntimes); 7] = [
            |r| r.kubeconfig_path = Some("kubeconfig".to_owned()),
            |r| r.kubeconfig_path = Some("/etc/../kubeconfig".to_owned()),
            |r| r.docker_socket = Some("docker.sock".to_owned()),
            |r| r.host_socket_dir = Some("run/pg".to_owned()),
            |r| r.host_bin_dir = Some("bin".to_owned()),
            |r| r.host_os_user = Some("Postgres".to_owned()),
            |r| r.host_port = Some(0),
        ];
        for change in refused {
            assert!(matches!(with(change), Err(ApiError::BadRequest(_))));
        }
        assert_eq!(
            with(|r| r.host_os_user = Some(" pg_admin-2 ".to_owned()))
                .unwrap()
                .host_os_user
                .as_deref(),
            Some("pg_admin-2")
        );
        assert!(!is_os_user_name(&"a".repeat(33)));
    }

    #[test]
    fn source_ids_must_fit_their_runtime() {
        let request = |runtime, namespace: &str, name: &str| UpdateAgentDatabaseSourceRequest {
            runtime,
            namespace: namespace.to_owned(),
            name: name.to_owned(),
            included: None,
        };
        assert!(source_id(&request(DatabaseRuntime::Kubernetes, "apps", "pg")).is_ok());
        assert!(source_id(&request(DatabaseRuntime::Kubernetes, "apps", "../x")).is_err());
        assert!(source_id(&request(DatabaseRuntime::Kubernetes, "Apps", "pg")).is_err());
        assert!(source_id(&request(DatabaseRuntime::Docker, "", "Wiki_DB")).is_ok());
        assert!(source_id(&request(DatabaseRuntime::Host, "", "postgresql")).is_ok());
        let Err(ApiError::BadRequest(message)) =
            source_id(&request(DatabaseRuntime::Host, "", "other"))
        else {
            panic!("a host source has one name");
        };
        assert!(message.contains("host"), "{message}");
    }

    #[test]
    fn an_undecided_source_follows_the_selection_mode() {
        let row = AgentDatabaseSourceRow {
            runtime: DatabaseRuntime::Kubernetes,
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
