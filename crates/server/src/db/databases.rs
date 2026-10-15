// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Queries for a host's database dump settings and for the database workloads
//! its agent found in its Kubernetes cluster. The settings live on the agent,
//! like virtual-machine staging: the cluster and the dump directory belong to
//! the host, and a schedule only opts in.

use chrono::{DateTime, Utc};
use shared::database::{
    DatabaseBackupConfig, DatabaseEngine, DatabaseSelectionMode, DatabaseSourceConfig,
    DatabaseSourceId, DatabaseSourceKind, DiscoveredDatabaseSource,
};
use sqlx::PgPool;

use crate::error::ApiError;

/// A host's dump settings as stored on its agent row.
#[derive(Debug, Clone)]
pub struct AgentDatabaseBackupRow {
    /// Whether this host dumps databases at all.
    pub database_backup_enabled: bool,
    /// Absolute directory that holds the dump pipes while a backup runs.
    pub database_dump_dir: String,
    /// Kubeconfig file on the agent host, or `None` for the agent's default.
    pub database_kubeconfig_path: Option<String>,
    /// Which direction the per-source flags are read in.
    pub database_selection: DatabaseSelectionMode,
}

/// New dump settings for a host. A `None` selection keeps the stored mode,
/// resolved inside the `UPDATE` so a concurrent save cannot be reverted.
#[derive(Debug, Clone, Copy)]
pub struct DatabaseBackupPatch<'a> {
    /// Whether this host dumps databases at all.
    pub enabled: bool,
    /// Absolute directory that holds the dump pipes.
    pub dump_dir: &'a str,
    /// Kubeconfig file on the agent host, or `None` for the agent's default.
    pub kubeconfig_path: Option<&'a str>,
    /// Which sources the per-source flags select, or `None` to keep the
    /// stored mode.
    pub selection: Option<DatabaseSelectionMode>,
}

/// One database workload of a host: what the last scan saw and the
/// operator's choice.
#[derive(Debug, Clone)]
pub struct AgentDatabaseSourceRow {
    /// Kubernetes namespace.
    pub namespace: String,
    /// The `CloudNativePG` cluster, or the annotated pod's workload.
    pub name: String,
    /// The operator's choice, or `None` when the host's selection decides.
    pub included: Option<bool>,
    /// How the last scan found it, `None` before any scan has.
    pub kind: Option<DatabaseSourceKind>,
    /// Its engine, `None` before any scan has seen it.
    pub engine: Option<DatabaseEngine>,
    /// The pod a dump would run in, as of the last scan.
    pub pod: Option<String>,
    /// The container inside that pod.
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

fn is_storable_database_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_DATABASE_NAME_LEN && !name.contains('\0')
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
    sqlx::query_as!(
        AgentDatabaseBackupRow,
        "SELECT database_backup_enabled, database_dump_dir, database_kubeconfig_path, \
         database_selection AS \"database_selection: DatabaseSelectionMode\" FROM agents WHERE id \
         = $1",
        agent_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent id '{agent_id}' not found")),
        other => ApiError::Database(other),
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
    sqlx::query_as!(
        AgentDatabaseBackupRow,
        "UPDATE agents SET database_backup_enabled = $2, database_dump_dir = $3, \
         database_kubeconfig_path = $4, database_selection = COALESCE($5, \
         agents.database_selection) WHERE id = $1 RETURNING database_backup_enabled, \
         database_dump_dir, database_kubeconfig_path, database_selection AS \"database_selection: \
         DatabaseSelectionMode\"",
        agent_id,
        patch.enabled,
        patch.dump_dir,
        patch.kubeconfig_path,
        patch.selection.map(|selection| selection.to_string()) as Option<String>,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent id '{agent_id}' not found")),
        other => ApiError::Database(other),
    })
}

/// Lists a host's database sources by namespace and name, so the table keeps
/// its order between scans.
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
        "SELECT namespace, name, included, kind AS \"kind: DatabaseSourceKind\", engine AS \
         \"engine: DatabaseEngine\", pod, container, databases, last_error, last_scanned_at FROM \
         agent_database_sources WHERE agent_id = $1 ORDER BY namespace, name",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Records the result of a scan: every reported source is inserted or
/// refreshed, and a source that is gone is dropped unless the operator made a
/// choice about it, which is kept so a database that is briefly down does not
/// lose its setting.
///
/// # Errors
///
/// Returns [`ApiError::Database`] when any of the statements fails.
pub async fn record_database_scan(
    pool: &PgPool,
    agent_id: i64,
    sources: &[DiscoveredDatabaseSource],
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    let mut seen: Vec<String> = Vec::with_capacity(sources.len());

    for source in sources {
        if !is_kube_name(&source.id.namespace)
            || !is_kube_name(&source.id.name)
            || !is_kube_name(&source.pod)
        {
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
            "INSERT INTO agent_database_sources (agent_id, namespace, name, kind, engine, pod, \
             container, databases, last_error, last_scanned_at) VALUES ($1, $2, $3, $4, $5, $6, \
             $7, $8, $9, NOW()) ON CONFLICT (agent_id, namespace, name) DO UPDATE SET kind = \
             EXCLUDED.kind, engine = EXCLUDED.engine, pod = EXCLUDED.pod, container = \
             EXCLUDED.container, databases = EXCLUDED.databases, last_error = \
             EXCLUDED.last_error, last_scanned_at = NOW()",
            agent_id,
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
        seen.push(format!("{}/{}", source.id.namespace, source.id.name));
    }

    sqlx::query!(
        "DELETE FROM agent_database_sources WHERE agent_id = $1 AND NOT (namespace || '/' || name \
         = ANY($2)) AND included IS NULL",
        agent_id,
        &seen,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    sqlx::query!(
        "UPDATE agent_database_sources SET pod = NULL, databases = '{}', last_error = 'not found \
         in the last scan' WHERE agent_id = $1 AND NOT (namespace || '/' || name = ANY($2))",
        agent_id,
        &seen,
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
        "INSERT INTO agent_database_sources (agent_id, namespace, name, included) VALUES ($1, $2, \
         $3, $4) ON CONFLICT (agent_id, namespace, name) DO UPDATE SET included = \
         EXCLUDED.included",
        agent_id,
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
        enabled: settings.database_backup_enabled,
        dump_dir: settings.database_dump_dir,
        kubeconfig_path: settings.database_kubeconfig_path,
        selection: settings.database_selection,
        sources: sources
            .into_iter()
            .filter(|source| source.included.is_some())
            .map(|source| DatabaseSourceConfig {
                id: DatabaseSourceId {
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
    fn database_names_may_hold_anything_but_a_nul() {
        assert!(is_storable_database_name("my db/x"));
        assert!(!is_storable_database_name(""));
        assert!(!is_storable_database_name("a\0b"));
        assert!(!is_storable_database_name(&"a".repeat(256)));
    }
}
