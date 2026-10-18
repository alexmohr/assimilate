// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Queries for a host's database dump settings and for the database workloads
//! its agent found: in its Kubernetes cluster, in Docker or Podman containers,
//! or on the host itself. The settings live on the agent, like virtual-machine
//! staging: the runtimes and the dump directory belong to the host, and a
//! schedule only opts in.

use chrono::{DateTime, Utc};
use shared::database::{
    DatabaseBackupConfig, DatabaseEngine, DatabaseRuntime, DatabaseRuntimes, DatabaseSelectionMode,
    DatabaseSourceConfig, DatabaseSourceId, DatabaseSourceKind, DiscoveredDatabaseSource,
    HOST_SOURCE_NAME,
};
use sqlx::PgPool;

use crate::error::ApiError;

/// A host's dump settings as stored on its agent row.
#[derive(Debug, Clone)]
pub struct AgentDatabaseBackupRow {
    /// Whether this host dumps databases at all.
    pub enabled: bool,
    /// Absolute directory that holds the dump pipes while a backup runs.
    pub dump_dir: String,
    /// Where the host looks for databases.
    pub runtimes: DatabaseRuntimes,
    /// Which direction the per-source flags are read in.
    pub selection: DatabaseSelectionMode,
}

/// New dump settings for a host. A `None` selection keeps the stored mode,
/// resolved inside the `UPDATE` so a concurrent save cannot be reverted.
#[derive(Debug, Clone, Copy)]
pub struct DatabaseBackupPatch<'a> {
    /// Whether this host dumps databases at all.
    pub enabled: bool,
    /// Absolute directory that holds the dump pipes.
    pub dump_dir: &'a str,
    /// Where the host looks for databases.
    pub runtimes: &'a DatabaseRuntimes,
    /// Which sources the per-source flags select, or `None` to keep the
    /// stored mode.
    pub selection: Option<DatabaseSelectionMode>,
}

/// One database workload of a host: what the last scan saw and the
/// operator's choice.
#[derive(Debug, Clone)]
pub struct AgentDatabaseSourceRow {
    /// Where the source runs.
    pub runtime: DatabaseRuntime,
    /// The Kubernetes namespace or Compose project, empty when there is none.
    pub namespace: String,
    /// The cluster, workload, container or Compose service, or `postgresql`
    /// for the host's own server.
    pub name: String,
    /// The operator's choice, or `None` when the host's selection decides.
    pub included: Option<bool>,
    /// How the last scan found it, `None` before any scan has.
    pub kind: Option<DatabaseSourceKind>,
    /// Its engine, `None` before any scan has seen it.
    pub engine: Option<DatabaseEngine>,
    /// Where a dump would run, as of the last scan.
    pub pod: Option<String>,
    /// The container inside that pod, or the OS user of a host dump.
    pub container: Option<String>,
    /// The databases a dump would include, as of the last scan.
    pub databases: Vec<String>,
    /// Why the last scan could not list the databases, when it could not.
    pub last_error: Option<String>,
    /// When the agent last reported this source.
    pub last_scanned_at: Option<DateTime<Utc>>,
}

/// Longest Kubernetes name or namespace this server stores. Kubernetes caps
/// names at 253 characters; the check keeps a misbehaving agent from writing
/// anything else.
const MAX_KUBE_NAME_LEN: usize = 253;

/// Longest container, Compose project or service name this server stores.
const MAX_CONTAINER_NAME_LEN: usize = 253;

/// Longest path this server stores for where a host dump runs.
const MAX_PATH_LEN: usize = 4096;

/// Longest database name this server stores. `PostgreSQL` truncates at 63
/// bytes, so anything near this is not a real name.
const MAX_DATABASE_NAME_LEN: usize = 255;

/// Most databases stored per source, so one report cannot grow a row without
/// bound.
const MAX_DATABASES_PER_SOURCE: usize = 1000;

/// Whether `name` is a Kubernetes object name: lower-case alphanumerics, `-`
/// and `.`, starting and ending with an alphanumeric.
#[must_use]
pub fn is_kube_name(name: &str) -> bool {
    let edge = |c: Option<char>| c.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    name.len() <= MAX_KUBE_NAME_LEN
        && edge(name.chars().next())
        && edge(name.chars().last())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
}

/// Whether `name` is a container, Compose project or service name: an
/// alphanumeric followed by alphanumerics, `_`, `.` and `-`.
#[must_use]
pub fn is_container_name(name: &str) -> bool {
    name.len() <= MAX_CONTAINER_NAME_LEN
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Whether `id` names a source the runtime it claims could hold.
#[must_use]
pub fn is_valid_source_id(id: &DatabaseSourceId) -> bool {
    match id.runtime {
        DatabaseRuntime::Kubernetes => is_kube_name(&id.namespace) && is_kube_name(&id.name),
        DatabaseRuntime::Docker => {
            (id.namespace.is_empty() || is_container_name(&id.namespace))
                && is_container_name(&id.name)
        }
        DatabaseRuntime::Host => id.namespace.is_empty() && id.name == HOST_SOURCE_NAME,
    }
}

/// Whether a scan's report of where `source` runs is one this server stores:
/// a pod or container name, or an absolute socket directory (or none) on the
/// host.
fn is_storable_place(source: &DiscoveredDatabaseSource) -> bool {
    let detail_ok =
        source.container.len() <= MAX_CONTAINER_NAME_LEN && !source.container.contains('\0');
    detail_ok
        && match source.id.runtime {
            DatabaseRuntime::Kubernetes => is_kube_name(&source.pod),
            DatabaseRuntime::Docker => is_container_name(&source.pod),
            DatabaseRuntime::Host => {
                source.pod.is_empty()
                    || (source.pod.starts_with('/')
                        && source.pod.len() <= MAX_PATH_LEN
                        && !source.pod.contains('\0'))
            }
        }
}

fn is_storable_database_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_DATABASE_NAME_LEN && !name.contains('\0')
}

/// A stored port, or `None` for one outside the range the column allows.
fn port(stored: Option<i32>) -> Option<u16> {
    stored.and_then(|port| u16::try_from(port).ok())
}

/// Reads a host's dump settings.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] when no agent has that id, or
/// [`ApiError::Database`] when the query fails.
pub async fn get_agent_database_backup(
    pool: &PgPool,
    agent_id: i64,
) -> Result<AgentDatabaseBackupRow, ApiError> {
    let row = sqlx::query!(
        "SELECT database_backup_enabled, database_dump_dir, database_selection AS \
         \"database_selection: DatabaseSelectionMode\", database_kubernetes_enabled, \
         database_kubeconfig_path, database_docker_enabled, database_docker_socket, \
         database_host_enabled, database_host_os_user, database_host_socket_dir, \
         database_host_port, database_host_bin_dir FROM agents WHERE id = $1",
        agent_id,
    )
    .fetch_one(pool)
    .await
    .map_err(super::agent_row_error(agent_id))?;
    Ok(AgentDatabaseBackupRow {
        enabled: row.database_backup_enabled,
        dump_dir: row.database_dump_dir,
        selection: row.database_selection,
        runtimes: DatabaseRuntimes {
            kubernetes: row.database_kubernetes_enabled,
            kubeconfig_path: row.database_kubeconfig_path,
            docker: row.database_docker_enabled,
            docker_socket: row.database_docker_socket,
            host: row.database_host_enabled,
            host_os_user: row.database_host_os_user,
            host_socket_dir: row.database_host_socket_dir,
            host_port: port(row.database_host_port),
            host_bin_dir: row.database_host_bin_dir,
        },
    })
}

/// Writes a host's dump settings.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] when no agent has that id, or
/// [`ApiError::Database`] when the update fails.
pub async fn update_agent_database_backup(
    pool: &PgPool,
    agent_id: i64,
    patch: DatabaseBackupPatch<'_>,
) -> Result<AgentDatabaseBackupRow, ApiError> {
    let runtimes = patch.runtimes;
    sqlx::query!(
        "UPDATE agents SET database_backup_enabled = $2, database_dump_dir = $3, \
         database_selection = COALESCE($4, agents.database_selection), \
         database_kubernetes_enabled = $5, database_kubeconfig_path = $6, database_docker_enabled \
         = $7, database_docker_socket = $8, database_host_enabled = $9, database_host_os_user = \
         $10, database_host_socket_dir = $11, database_host_port = $12, database_host_bin_dir = \
         $13 WHERE id = $1 RETURNING id",
        agent_id,
        patch.enabled,
        patch.dump_dir,
        patch.selection.map(|selection| selection.to_string()) as Option<String>,
        runtimes.kubernetes,
        runtimes.kubeconfig_path.as_deref(),
        runtimes.docker,
        runtimes.docker_socket.as_deref(),
        runtimes.host,
        runtimes.host_os_user.as_deref(),
        runtimes.host_socket_dir.as_deref(),
        runtimes.host_port.map(i32::from),
        runtimes.host_bin_dir.as_deref(),
    )
    .fetch_one(pool)
    .await
    .map_err(super::agent_row_error(agent_id))?;
    get_agent_database_backup(pool, agent_id).await
}

/// Lists a host's database sources by runtime, namespace and name, so the
/// table keeps its order between scans.
///
/// # Errors
///
/// Returns [`ApiError::Database`] when the query fails.
pub async fn list_agent_database_sources(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<AgentDatabaseSourceRow>, ApiError> {
    sqlx::query_as!(
        AgentDatabaseSourceRow,
        "SELECT runtime AS \"runtime: DatabaseRuntime\", namespace, name, included, kind AS \
         \"kind: DatabaseSourceKind\", engine AS \"engine: DatabaseEngine\", pod, container, \
         databases, last_error, last_scanned_at FROM agent_database_sources WHERE agent_id = $1 \
         ORDER BY runtime, namespace, name",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// The key a source is told apart by within one host, as the scan
/// reconciliation compares it.
fn source_key(id: &DatabaseSourceId) -> String {
    format!("{}:{}/{}", id.runtime, id.namespace, id.name)
}

/// Records the result of a scan: every reported source is inserted or
/// refreshed, and a source that is gone is dropped unless the operator made a
/// choice about it, which is kept so a database that is briefly down does not
/// lose its setting. Sources in `unsearched` runtimes are left exactly as they
/// were: the scan could not search them, so their absence says nothing.
///
/// # Errors
///
/// Returns [`ApiError::Database`] when any of the statements fails.
pub async fn record_database_scan(
    pool: &PgPool,
    agent_id: i64,
    sources: &[DiscoveredDatabaseSource],
    unsearched: &[DatabaseRuntime],
) -> Result<(), ApiError> {
    let unsearched: Vec<String> = unsearched.iter().map(ToString::to_string).collect();
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    let mut seen: Vec<String> = Vec::with_capacity(sources.len());

    for source in sources {
        if !is_valid_source_id(&source.id) || !is_storable_place(source) {
            tracing::warn!(
                agent_id,
                "an agent reported a database source with a name this server will not store, \
                 skipping it"
            );
            continue;
        }
        let databases: Vec<String> = source
            .databases
            .iter()
            .filter(|name| is_storable_database_name(name))
            .take(MAX_DATABASES_PER_SOURCE)
            .cloned()
            .collect();
        sqlx::query!(
            "INSERT INTO agent_database_sources (agent_id, runtime, namespace, name, kind, \
             engine, pod, container, databases, last_error, last_scanned_at) VALUES ($1, $2, $3, \
             $4, $5, $6, $7, $8, $9, $10, NOW()) ON CONFLICT (agent_id, runtime, namespace, name) \
             DO UPDATE SET kind = EXCLUDED.kind, engine = EXCLUDED.engine, pod = EXCLUDED.pod, \
             container = EXCLUDED.container, databases = EXCLUDED.databases, last_error = \
             EXCLUDED.last_error, last_scanned_at = NOW()",
            agent_id,
            source.id.runtime.to_string(),
            source.id.namespace,
            source.id.name,
            source.kind.to_string(),
            source.engine.to_string(),
            source.pod,
            source.container,
            &databases,
            source.error.as_deref(),
        )
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;
        seen.push(source_key(&source.id));
    }

    sqlx::query!(
        "DELETE FROM agent_database_sources WHERE agent_id = $1 AND NOT (runtime || ':' || \
         namespace || '/' || name = ANY($2)) AND included IS NULL AND NOT (runtime = ANY($3))",
        agent_id,
        &seen,
        &unsearched,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    sqlx::query!(
        "UPDATE agent_database_sources SET pod = NULL, databases = '{}', last_error = 'not found \
         in the last scan' WHERE agent_id = $1 AND NOT (runtime || ':' || namespace || '/' || \
         name = ANY($2)) AND NOT (runtime = ANY($3))",
        agent_id,
        &seen,
        &unsearched,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    tx.commit().await.map_err(ApiError::Database)
}

/// Records the operator's choice for one source. The row is created when no
/// scan has seen the source yet, so a source can be chosen ahead of its first
/// scan. `None` clears the choice, handing the source back to the host's
/// selection mode.
///
/// # Errors
///
/// Returns [`ApiError::Database`] when the upsert fails.
pub async fn set_database_source_included(
    pool: &PgPool,
    agent_id: i64,
    id: &DatabaseSourceId,
    included: Option<bool>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO agent_database_sources (agent_id, runtime, namespace, name, included) VALUES \
         ($1, $2, $3, $4, $5) ON CONFLICT (agent_id, runtime, namespace, name) DO UPDATE SET \
         included = EXCLUDED.included",
        agent_id,
        id.runtime.to_string(),
        id.namespace,
        id.name,
        included,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Assembles the dump configuration delivered to an agent: the host's
/// settings plus the choices the operator made per source.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] when no agent has that id, or
/// [`ApiError::Database`] when a query fails.
pub async fn load_config(pool: &PgPool, agent_id: i64) -> Result<DatabaseBackupConfig, ApiError> {
    let settings = get_agent_database_backup(pool, agent_id).await?;
    let sources = list_agent_database_sources(pool, agent_id).await?;
    Ok(DatabaseBackupConfig {
        enabled: settings.enabled,
        dump_dir: settings.dump_dir,
        runtimes: settings.runtimes,
        selection: settings.selection,
        sources: sources
            .into_iter()
            .filter(|source| source.included.is_some())
            .map(|source| DatabaseSourceConfig {
                id: DatabaseSourceId {
                    runtime: source.runtime,
                    namespace: source.namespace,
                    name: source.name,
                },
                included: source.included,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kubernetes_names_are_checked_like_kubernetes_does() {
        assert!(is_kube_name("pg"));
        assert!(is_kube_name("shop-db.v2"));
        assert!(is_kube_name("0abc9"));
        assert!(!is_kube_name(""));
        assert!(!is_kube_name("-pg"));
        assert!(!is_kube_name("pg-"));
        assert!(!is_kube_name("Shop"));
        assert!(!is_kube_name("a/b"));
        assert!(!is_kube_name("a b"));
        assert!(!is_kube_name(&"a".repeat(254)));
        assert!(is_kube_name(&"a".repeat(253)));
    }

    #[test]
    fn container_names_follow_the_engines_rules() {
        assert!(is_container_name("wiki-db"));
        assert!(is_container_name("Shop_DB.2"));
        assert!(!is_container_name(""));
        assert!(!is_container_name("-db"));
        assert!(!is_container_name("a/b"));
        assert!(!is_container_name(&"a".repeat(254)));
    }

    fn id(runtime: DatabaseRuntime, namespace: &str, name: &str) -> DatabaseSourceId {
        DatabaseSourceId {
            runtime,
            namespace: namespace.to_owned(),
            name: name.to_owned(),
        }
    }

    #[test]
    fn each_runtime_checks_its_own_names() {
        assert!(is_valid_source_id(&id(
            DatabaseRuntime::Kubernetes,
            "shop",
            "pg"
        )));
        assert!(!is_valid_source_id(&id(
            DatabaseRuntime::Kubernetes,
            "",
            "pg"
        )));
        assert!(is_valid_source_id(&id(
            DatabaseRuntime::Docker,
            "",
            "Wiki_DB"
        )));
        assert!(is_valid_source_id(&id(
            DatabaseRuntime::Docker,
            "shop",
            "db"
        )));
        assert!(!is_valid_source_id(&id(
            DatabaseRuntime::Docker,
            "a b",
            "db"
        )));
        assert!(is_valid_source_id(&id(
            DatabaseRuntime::Host,
            "",
            HOST_SOURCE_NAME
        )));
        assert!(!is_valid_source_id(&id(
            DatabaseRuntime::Host,
            "x",
            HOST_SOURCE_NAME
        )));
        assert!(!is_valid_source_id(&id(DatabaseRuntime::Host, "", "other")));
    }

    #[test]
    fn where_a_dump_runs_is_checked_per_runtime() {
        let source = |runtime, pod: &str, container: &str| DiscoveredDatabaseSource {
            id: id(runtime, "", "x"),
            kind: DatabaseSourceKind::Host,
            engine: DatabaseEngine::Postgresql,
            pod: pod.to_owned(),
            container: container.to_owned(),
            databases: Vec::new(),
            error: None,
        };
        assert!(is_storable_place(&source(
            DatabaseRuntime::Kubernetes,
            "pg-1",
            "postgres"
        )));
        assert!(!is_storable_place(&source(
            DatabaseRuntime::Kubernetes,
            "Pg",
            "postgres"
        )));
        assert!(is_storable_place(&source(
            DatabaseRuntime::Docker,
            "wiki-db",
            ""
        )));
        assert!(!is_storable_place(&source(
            DatabaseRuntime::Docker,
            "/wiki",
            ""
        )));
        assert!(is_storable_place(&source(
            DatabaseRuntime::Host,
            "",
            "postgres"
        )));
        assert!(is_storable_place(&source(
            DatabaseRuntime::Host,
            "/run/pg",
            "postgres"
        )));
        assert!(!is_storable_place(&source(
            DatabaseRuntime::Host,
            "run/pg",
            "postgres"
        )));
        assert!(!is_storable_place(&source(
            DatabaseRuntime::Host,
            "",
            "a\0b"
        )));
    }

    #[test]
    fn a_stored_port_outside_the_range_reads_as_none() {
        assert_eq!(port(Some(5433)), Some(5433));
        assert_eq!(port(Some(70000)), None);
        assert_eq!(port(None), None);
    }

    #[test]
    fn database_names_may_hold_anything_but_a_nul() {
        assert!(is_storable_database_name("my db/x"));
        assert!(!is_storable_database_name(""));
        assert!(!is_storable_database_name("a\0b"));
        assert!(!is_storable_database_name(&"a".repeat(256)));
    }
}
