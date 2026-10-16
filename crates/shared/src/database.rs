// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Types describing the databases an agent dumps into a backup: the engines it
//! knows, the runtimes it finds them in (a Kubernetes cluster, Docker or Podman
//! containers, or the host itself), the per-host settings that choose which
//! ones are dumped, and the manifest written into each archive.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Prefix of every pod annotation and container label the agent reads. A
/// domain the project owns through GitHub Pages, so it never collides with
/// another tool's keys.
pub const ANNOTATION_PREFIX: &str = "assimilate.alexmohr.github.io";

/// Annotation (on a pod) or label (on a container) naming the engine of a
/// database the operator marked by hand.
pub const ANNOTATION_DATABASE: &str = "assimilate.alexmohr.github.io/database";

/// Annotation naming the container the dump runs in. Without it, the pod's
/// first container is used.
pub const ANNOTATION_CONTAINER: &str = "assimilate.alexmohr.github.io/container";

/// Annotation or label naming the database role the dump connects as.
pub const ANNOTATION_USER: &str = "assimilate.alexmohr.github.io/user";

/// Annotation or label listing the databases to dump, comma separated.
/// Without it, every database that accepts connections is dumped.
pub const ANNOTATION_DATABASES: &str = "assimilate.alexmohr.github.io/databases";

/// Role a dump connects as when the pod or container does not name one. Both
/// the official `postgres` image and `CloudNativePG` let it in over the local
/// socket. It is also the OS user a host dump runs as by default, which
/// peer authentication maps to the role of the same name.
pub const DEFAULT_POSTGRES_USER: &str = "postgres";

/// Name of the one `PostgreSQL` instance a host source stands for.
pub const HOST_SOURCE_NAME: &str = "postgresql";

/// Directory the agent creates its dump pipes and manifest in when the
/// operator leaves it empty. Never below one of [`EXCLUDED_WHILE_DUMPING`],
/// which a run that dumps databases leaves out of the archive.
pub const DEFAULT_DUMP_DIR: &str = "/var/lib/assimilate/database-dumps";

/// Directories a run that dumps databases leaves out of its archive. Such a
/// run makes borg read special files as if they were regular ones, and these
/// hold the device nodes and kernel interfaces that would hang it or fill the
/// archive.
pub const EXCLUDED_WHILE_DUMPING: [&str; 4] = ["/dev", "/proc", "/sys", "/run"];

/// Path inside every archive that lists the database dumps it holds, relative
/// to the directory the dumps were streamed from.
pub const MANIFEST_FILE_NAME: &str = "databases.json";

/// Version of the manifest format, so a later reader can tell old archives
/// apart.
pub const MANIFEST_VERSION: u32 = 1;

/// A database engine the agent can dump.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    TS,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum DatabaseEngine {
    /// `PostgreSQL`, dumped with `pg_dump` in custom format.
    Postgresql,
}

impl DatabaseEngine {
    /// File extension of one dump of this engine inside the archive.
    #[must_use]
    pub const fn dump_extension(self) -> &'static str {
        match self {
            Self::Postgresql => "pgdump",
        }
    }

    /// Format of one dump of this engine, as recorded in the manifest.
    #[must_use]
    pub const fn dump_format(self) -> DumpFormat {
        match self {
            Self::Postgresql => DumpFormat::PgCustom,
        }
    }
}

/// How a database dump is encoded, so a restore knows which tool reads it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema, strum_macros::Display,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum DumpFormat {
    /// `pg_dump --format=custom`, read back with `pg_restore`.
    PgCustom,
}

/// Where a database source runs, which decides how the agent reaches it.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Serialize,
    Deserialize,
    TS,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum DatabaseRuntime {
    /// A pod in the host's Kubernetes cluster, reached through the exec API.
    #[default]
    Kubernetes,
    /// A Docker or Podman container, reached through the engine's API socket.
    Docker,
    /// `PostgreSQL` installed on the host, run as a local OS user.
    Host,
}

/// How a database workload was found.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    TS,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum DatabaseSourceKind {
    /// A `CloudNativePG` cluster, found by its `cnpg.io/cluster` label.
    CnpgCluster,
    /// A pod carrying the `assimilate.alexmohr.github.io/database` annotation.
    Annotated,
    /// A Docker or Podman container carrying the
    /// `assimilate.alexmohr.github.io/database` label.
    Container,
    /// `PostgreSQL` installed on the host itself.
    Host,
}

/// Which database sources the per-source flags select.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    TS,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum DatabaseSelectionMode {
    /// Dump every source the agent finds except the ones turned off. A
    /// database deployed after the last scan is backed up rather than missed,
    /// which is why this is the default.
    #[default]
    All,
    /// Dump only the sources turned on.
    Selected,
}

impl DatabaseSelectionMode {
    /// Whether a source with no flag of its own is dumped under this mode.
    #[must_use]
    pub const fn includes_untouched(self) -> bool {
        match self {
            Self::All => true,
            Self::Selected => false,
        }
    }
}

/// Where a database source lives: unique per host. A pod or container would
/// change with every restart, so the workload or service is named instead.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS, ToSchema,
)]
#[ts(export)]
pub struct DatabaseSourceId {
    /// Where the source runs.
    #[serde(default)]
    pub runtime: DatabaseRuntime,
    /// The Kubernetes namespace, or the Compose project of a container.
    /// Empty for a container outside a Compose project and for the host.
    #[serde(default)]
    pub namespace: String,
    /// The `CloudNativePG` cluster, the annotated pod's owning workload, the
    /// container's Compose service or name, or [`HOST_SOURCE_NAME`].
    pub name: String,
}

impl std::fmt::Display for DatabaseSourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.runtime {
            DatabaseRuntime::Kubernetes => write!(f, "{}/{}", self.namespace, self.name),
            DatabaseRuntime::Docker | DatabaseRuntime::Host if self.namespace.is_empty() => {
                write!(f, "{}:{}", self.runtime, self.name)
            }
            DatabaseRuntime::Docker | DatabaseRuntime::Host => {
                write!(f, "{}:{}/{}", self.runtime, self.namespace, self.name)
            }
        }
    }
}

/// A database workload as the agent last saw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct DiscoveredDatabaseSource {
    /// Where the source lives.
    #[serde(flatten)]
    #[ts(flatten)]
    pub id: DatabaseSourceId,
    /// How it was found.
    pub kind: DatabaseSourceKind,
    /// Its engine.
    pub engine: DatabaseEngine,
    /// Where a dump would run right now: the pod, the container's name, or
    /// the socket directory on the host (empty for the default one).
    pub pod: String,
    /// The container inside that pod, or the OS user a host dump runs as.
    /// Empty for a container, which is its own place to run.
    pub container: String,
    /// The databases a dump would include.
    pub databases: Vec<String>,
    /// Why the databases could not be listed, when they could not.
    #[serde(default)]
    pub error: Option<String>,
}

/// The operator's choice for one source, delivered to the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseSourceConfig {
    /// Which source this is.
    #[serde(flatten)]
    pub id: DatabaseSourceId,
    /// Whether it is dumped, or `None` when nobody decided and the host's
    /// [`DatabaseBackupConfig::selection`] answers instead.
    #[serde(default)]
    pub included: Option<bool>,
}

/// Which runtimes a host looks for databases in, and how it reaches each.
/// Every one the host turns on must be reachable for a backup that dumps
/// databases to run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct DatabaseRuntimes {
    /// Whether the host's Kubernetes cluster is searched.
    #[serde(default)]
    pub kubernetes: bool,
    /// Kubeconfig file on the agent host. `None` uses the in-cluster service
    /// account, or `$KUBECONFIG` and `~/.kube/config`.
    #[serde(default)]
    pub kubeconfig_path: Option<String>,
    /// Whether Docker or Podman containers are searched.
    #[serde(default)]
    pub docker: bool,
    /// The engine's API socket. `None` uses `$DOCKER_HOST`, or
    /// `/var/run/docker.sock`.
    #[serde(default)]
    pub docker_socket: Option<String>,
    /// Whether `PostgreSQL` installed on the host is dumped.
    #[serde(default)]
    pub host: bool,
    /// The OS user host dumps run as, `None` for [`DEFAULT_POSTGRES_USER`].
    /// Peer authentication lets it in as the role of the same name; a
    /// password setup reads that user's `~/.pgpass`.
    #[serde(default)]
    pub host_os_user: Option<String>,
    /// Directory of the server's Unix socket, `None` for the client's
    /// default.
    #[serde(default)]
    pub host_socket_dir: Option<String>,
    /// The server's port, `None` for the client's default (5432).
    #[serde(default)]
    pub host_port: Option<u16>,
    /// Directory holding `pg_dump` and `psql`, `None` to find them on the
    /// agent's `PATH`. Lets a host with several versions installed side by
    /// side pick the one matching its server.
    #[serde(default)]
    pub host_bin_dir: Option<String>,
}

impl DatabaseRuntimes {
    /// Whether any runtime is turned on.
    #[must_use]
    pub const fn any(&self) -> bool {
        self.kubernetes || self.docker || self.host
    }

    /// The OS user host dumps run as.
    #[must_use]
    pub fn host_os_user(&self) -> &str {
        self.host_os_user
            .as_deref()
            .unwrap_or(DEFAULT_POSTGRES_USER)
    }
}

/// A host's database dump settings, delivered to the agent as part of its
/// configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseBackupConfig {
    /// Whether this host dumps databases at all. A schedule that asks for them
    /// while this is off backs up none of them.
    pub enabled: bool,
    /// Absolute directory on the agent host that holds the dump pipes and the
    /// manifest while a backup runs. It is what the archive stores them under.
    #[serde(default = "default_dump_dir")]
    pub dump_dir: String,
    /// Where the host looks for databases.
    #[serde(flatten)]
    pub runtimes: DatabaseRuntimes,
    /// Which sources the per-source flags select.
    #[serde(default)]
    pub selection: DatabaseSelectionMode,
    /// Per-source flags, for the sources the operator has touched.
    #[serde(default)]
    pub sources: Vec<DatabaseSourceConfig>,
}

fn default_dump_dir() -> String {
    DEFAULT_DUMP_DIR.to_owned()
}

impl Default for DatabaseBackupConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dump_dir: default_dump_dir(),
            runtimes: DatabaseRuntimes::default(),
            selection: DatabaseSelectionMode::default(),
            sources: Vec::new(),
        }
    }
}

/// Whether `dir` is somewhere the agent may create dump pipes: an absolute
/// path, not below a directory a dumping run leaves out of the archive, and
/// without `..` components that could step back into one.
#[must_use]
pub fn is_valid_dump_dir(dir: &str) -> bool {
    let path = std::path::Path::new(dir);
    path.is_absolute()
        && path
            .components()
            .all(|component| !matches!(component, std::path::Component::ParentDir))
        && path.parent().is_some()
        && !EXCLUDED_WHILE_DUMPING
            .iter()
            .any(|excluded| path.starts_with(excluded))
}

impl DatabaseBackupConfig {
    /// Whether a source is dumped, under whichever direction
    /// [`DatabaseBackupConfig::selection`] reads the flags in.
    #[must_use]
    pub fn includes(&self, id: &DatabaseSourceId) -> bool {
        self.sources
            .iter()
            .find(|source| &source.id == id)
            .and_then(|source| source.included)
            .unwrap_or_else(|| self.selection.includes_untouched())
    }
}

/// One dump inside an archive, as listed in its manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct DatabaseManifestEntry {
    /// The source the dump came from.
    #[serde(flatten)]
    #[ts(flatten)]
    pub source: DatabaseSourceId,
    /// How the source was found.
    pub kind: DatabaseSourceKind,
    /// Its engine.
    pub engine: DatabaseEngine,
    /// Where the dump ran: the pod, the container, or the host's socket
    /// directory.
    pub pod: String,
    /// The database dumped.
    pub database: String,
    /// The dump's path in the archive, relative to the manifest.
    pub path: String,
    /// How the dump is encoded.
    pub format: DumpFormat,
}

/// The list of database dumps an archive holds. Restores read it rather than
/// the server's tables, so an archive explains itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct DatabaseManifest {
    /// Format version, [`MANIFEST_VERSION`] for archives written by this build.
    pub version: u32,
    /// The dumps, in the order they were written.
    pub dumps: Vec<DatabaseManifestEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(namespace: &str, name: &str) -> DatabaseSourceId {
        DatabaseSourceId {
            runtime: DatabaseRuntime::Kubernetes,
            namespace: namespace.to_owned(),
            name: name.to_owned(),
        }
    }

    fn config(selection: DatabaseSelectionMode) -> DatabaseBackupConfig {
        DatabaseBackupConfig {
            enabled: true,
            dump_dir: DEFAULT_DUMP_DIR.to_owned(),
            runtimes: DatabaseRuntimes::default(),
            selection,
            sources: vec![
                DatabaseSourceConfig {
                    id: id("apps", "on"),
                    included: Some(true),
                },
                DatabaseSourceConfig {
                    id: id("apps", "off"),
                    included: Some(false),
                },
            ],
        }
    }

    #[test]
    fn a_flag_wins_over_the_selection_mode() {
        let all = config(DatabaseSelectionMode::All);
        assert!(all.includes(&id("apps", "on")));
        assert!(!all.includes(&id("apps", "off")));
        assert!(all.includes(&id("apps", "new")));

        let selected = config(DatabaseSelectionMode::Selected);
        assert!(selected.includes(&id("apps", "on")));
        assert!(!selected.includes(&id("apps", "off")));
        assert!(!selected.includes(&id("apps", "new")));
    }

    #[test]
    fn a_source_is_told_apart_by_its_namespace_and_runtime() {
        let selected = config(DatabaseSelectionMode::Selected);
        assert!(!selected.includes(&id("other", "on")));
        let in_docker = DatabaseSourceId {
            runtime: DatabaseRuntime::Docker,
            ..id("apps", "on")
        };
        assert!(!selected.includes(&in_docker));
    }

    #[test]
    fn each_runtime_names_its_sources_its_own_way() {
        let source = |runtime, namespace: &str, name: &str| {
            DatabaseSourceId {
                runtime,
                namespace: namespace.to_owned(),
                name: name.to_owned(),
            }
            .to_string()
        };
        assert_eq!(source(DatabaseRuntime::Kubernetes, "shop", "pg"), "shop/pg");
        assert_eq!(
            source(DatabaseRuntime::Docker, "", "wiki-db"),
            "docker:wiki-db"
        );
        assert_eq!(
            source(DatabaseRuntime::Docker, "wiki", "db"),
            "docker:wiki/db"
        );
        assert_eq!(
            source(DatabaseRuntime::Host, "", HOST_SOURCE_NAME),
            "host:postgresql"
        );
        assert_eq!(source(DatabaseRuntime::Host, "x", "y"), "host:x/y");
    }

    #[test]
    fn a_source_id_without_a_runtime_is_a_kubernetes_one() {
        let id: DatabaseSourceId = serde_json::from_str(r#"{"namespace":"a","name":"b"}"#).unwrap();
        assert_eq!(id.runtime, DatabaseRuntime::Kubernetes);
        let host: DatabaseSourceId =
            serde_json::from_str(r#"{"runtime":"host","name":"postgresql"}"#).unwrap();
        assert_eq!(host.namespace, "");
    }

    #[test]
    fn runtimes_say_whether_any_is_on_and_who_host_dumps_run_as() {
        let mut runtimes = DatabaseRuntimes::default();
        assert!(!runtimes.any());
        assert_eq!(runtimes.host_os_user(), DEFAULT_POSTGRES_USER);
        let turn_ons: [fn(&mut DatabaseRuntimes); 3] = [
            |r| r.kubernetes = true,
            |r| r.docker = true,
            |r| r.host = true,
        ];
        for turn_on in turn_ons {
            let mut one = DatabaseRuntimes::default();
            turn_on(&mut one);
            assert!(one.any());
        }
        runtimes.host_os_user = Some("pgadmin".to_owned());
        assert_eq!(runtimes.host_os_user(), "pgadmin");
    }

    #[test]
    fn dump_dirs_stay_out_of_the_excluded_trees() {
        assert!(is_valid_dump_dir(DEFAULT_DUMP_DIR));
        assert!(is_valid_dump_dir("/srv/dumps"));
        assert!(is_valid_dump_dir("/runner/dumps"));
        assert!(!is_valid_dump_dir("relative/dumps"));
        assert!(!is_valid_dump_dir("/"));
        assert!(!is_valid_dump_dir("/run/assimilate"));
        assert!(!is_valid_dump_dir("/dev"));
        assert!(!is_valid_dump_dir("/srv/../run/x"));
    }

    #[test]
    fn a_config_without_a_dump_dir_gets_the_default() {
        let config: DatabaseBackupConfig =
            serde_json::from_str(r#"{"enabled":true}"#).unwrap_or_default();
        assert!(config.enabled);
        assert_eq!(config.dump_dir, DEFAULT_DUMP_DIR);
        assert_eq!(config.runtimes, DatabaseRuntimes::default());
        assert_eq!(DatabaseBackupConfig::default().dump_dir, DEFAULT_DUMP_DIR);
    }

    #[test]
    fn engines_round_trip_through_their_wire_names() {
        assert_eq!(DatabaseEngine::Postgresql.to_string(), "postgresql");
        assert_eq!(
            "postgresql".parse::<DatabaseEngine>().ok(),
            Some(DatabaseEngine::Postgresql)
        );
        assert!("oracle".parse::<DatabaseEngine>().is_err());
        assert_eq!(DatabaseEngine::Postgresql.dump_extension(), "pgdump");
        assert_eq!(
            DatabaseEngine::Postgresql.dump_format(),
            DumpFormat::PgCustom
        );
    }

    #[test]
    fn a_manifest_entry_names_its_source_flat() {
        let entry = DatabaseManifestEntry {
            source: id("apps", "pg"),
            kind: DatabaseSourceKind::CnpgCluster,
            engine: DatabaseEngine::Postgresql,
            pod: "pg-2".to_owned(),
            database: "shop".to_owned(),
            path: "apps/pg/shop.pgdump".to_owned(),
            format: DumpFormat::PgCustom,
        };
        let json = serde_json::to_value(&entry).unwrap_or_default();
        assert_eq!(
            json.get("runtime").and_then(|v| v.as_str()),
            Some("kubernetes")
        );
        assert_eq!(json.get("namespace").and_then(|v| v.as_str()), Some("apps"));
        assert_eq!(json.get("name").and_then(|v| v.as_str()), Some("pg"));
        assert_eq!(
            json.get("kind").and_then(|v| v.as_str()),
            Some("cnpg_cluster")
        );
        assert_eq!(
            json.get("format").and_then(|v| v.as_str()),
            Some("pg_custom")
        );
        assert_eq!(id("apps", "pg").to_string(), "apps/pg");
    }
}
