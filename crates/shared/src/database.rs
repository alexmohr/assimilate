// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Types describing the databases an agent dumps into a backup: the engines it
//! knows, the Kubernetes workloads it finds them in, the per-host settings that
//! choose which ones are dumped, and the manifest written into each archive.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Prefix of every pod annotation the agent reads. A domain the project owns
/// through GitHub Pages, so it never collides with another tool's keys.
pub const ANNOTATION_PREFIX: &str = "assimilate.alexmohr.github.io";

/// Annotation naming the engine of a hand-written database pod.
pub const ANNOTATION_DATABASE: &str = "assimilate.alexmohr.github.io/database";

/// Annotation naming the container the dump runs in. Without it, the pod's
/// first container is used.
pub const ANNOTATION_CONTAINER: &str = "assimilate.alexmohr.github.io/container";

/// Annotation naming the database role the dump connects as.
pub const ANNOTATION_USER: &str = "assimilate.alexmohr.github.io/user";

/// Annotation listing the databases to dump, comma separated. Without it,
/// every database that accepts connections is dumped.
pub const ANNOTATION_DATABASES: &str = "assimilate.alexmohr.github.io/databases";

/// Role a dump connects as when the pod does not name one. Both the official
/// `postgres` image and `CloudNativePG` let it in over the local socket.
pub const DEFAULT_POSTGRES_USER: &str = "postgres";

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

/// How a database workload was found in the cluster.
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

/// Where a database source lives: unique per cluster. A pod name would change
/// with every restart, so the workload is named instead.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS, ToSchema,
)]
#[ts(export)]
pub struct DatabaseSourceId {
    /// Kubernetes namespace.
    pub namespace: String,
    /// The `CloudNativePG` cluster, or the annotated pod's owning workload.
    pub name: String,
}

impl std::fmt::Display for DatabaseSourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.namespace, self.name)
    }
}

/// A database workload as the agent last saw it in the cluster.
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
    /// The pod a dump would run in right now.
    pub pod: String,
    /// The container inside that pod.
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
    /// Kubeconfig file on the agent host. `None` uses the in-cluster service
    /// account, or `$KUBECONFIG` and `~/.kube/config`.
    #[serde(default)]
    pub kubeconfig_path: Option<String>,
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
            kubeconfig_path: None,
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
    /// The pod the dump ran in.
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
            namespace: namespace.to_owned(),
            name: name.to_owned(),
        }
    }

    fn config(selection: DatabaseSelectionMode) -> DatabaseBackupConfig {
        DatabaseBackupConfig {
            enabled: true,
            dump_dir: DEFAULT_DUMP_DIR.to_owned(),
            kubeconfig_path: None,
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
    fn a_source_is_told_apart_by_its_namespace() {
        assert!(!config(DatabaseSelectionMode::Selected).includes(&id("other", "on")));
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
